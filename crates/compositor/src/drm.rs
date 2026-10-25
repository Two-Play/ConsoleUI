//! Session backend: DRM/KMS on the primary GPU, libinput for keyboard and mouse, libseat (logind)
//! for device access and VT switching. Single GPU, single output (the first connected
//! connector; another one takes over when it is unplugged).
//!
//! Modelled on anvil's `udev.rs` (MIT, Copyright (c) 2017 Victor Berger), without multi-GPU,
//! DRM leasing and the debug overlays.

use std::path::{Path, PathBuf};
use std::time::Duration;

use smithay::backend::allocator::format::FormatSet;
use smithay::backend::allocator::gbm::{GbmAllocator, GbmBufferFlags, GbmDevice};
use smithay::backend::allocator::{Fourcc, Modifier};
use smithay::backend::drm::compositor::FrameFlags;
use smithay::backend::drm::exporter::gbm::GbmFramebufferExporter;
use smithay::backend::drm::output::{DrmOutput, DrmOutputManager, DrmOutputRenderElements};
use smithay::backend::drm::{DrmDevice, DrmDeviceFd, DrmEvent, DrmEventMetadata, DrmEventTime, DrmNode, NodeType};
use smithay::backend::egl::context::ContextPriority;
use smithay::backend::egl::{EGLContext, EGLDevice, EGLDisplay};
use smithay::backend::input::InputEvent;
use smithay::backend::libinput::{LibinputInputBackend, LibinputSessionInterface};
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::renderer::{ImportDma, ImportEgl, ImportMemWl};
use smithay::backend::session::libseat::LibSeatSession;
use smithay::backend::session::{Event as SessionEvent, Session};
use smithay::backend::udev::{UdevBackend, UdevEvent, all_gpus, primary_gpu};
use smithay::desktop::utils::OutputPresentationFeedback;
use smithay::input::keyboard::LedState;
use smithay::output::{Mode as WlMode, Output, PhysicalProperties};
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay::reexports::calloop::{EventLoop, RegistrationToken};
use smithay::reexports::drm::Device as _;
use smithay::reexports::drm::control::{Mode as DrmMode, ModeTypeFlags, connector, crtc};
use smithay::reexports::input::{DeviceCapability, Libinput};
use smithay::reexports::rustix::fs::OFlags;
use smithay::reexports::wayland_protocols::wp::presentation_time::server::wp_presentation_feedback;
use smithay::reexports::wayland_server::Display;
use smithay::reexports::wayland_server::backend::GlobalId;
use smithay::utils::{DeviceFd, Transform};
use smithay::wayland::dmabuf::DmabufFeedbackBuilder;
use smithay::wayland::drm_syncobj::{DrmSyncobjState, supports_syncobj_eventfd};
use smithay::wayland::presentation::Refresh;
use smithay_drm_extras::display_info;
use smithay_drm_extras::drm_scanner::{DrmScanEvent, DrmScanner};
use tracing::{error, info, warn};

use crate::backend::Backend;
use crate::options::Options;
use crate::render::{self, OutputElement};
use crate::state::State;

/// 8-bit formats only: widely supported, and the screenshot read-back stays simple.
const COLOR_FORMATS: &[Fourcc] = &[Fourcc::Abgr8888, Fourcc::Argb8888];

type GbmDrmOutputManager = DrmOutputManager<
    GbmAllocator<DrmDeviceFd>,
    GbmFramebufferExporter<DrmDeviceFd>,
    Option<OutputPresentationFeedback>,
    DrmDeviceFd,
>;
type GbmDrmOutput = DrmOutput<
    GbmAllocator<DrmDeviceFd>,
    GbmFramebufferExporter<DrmDeviceFd>,
    Option<OutputPresentationFeedback>,
    DrmDeviceFd,
>;

pub struct DrmData {
    session: LibSeatSession,
    /// Path of the GPU we drive (the seat's primary GPU)
    gpu_path: PathBuf,
    device: Option<Gpu>,
    keyboards: Vec<smithay::reexports::input::Device>,
    /// A render is already scheduled (avoids piling up timers)
    render_scheduled: bool,
}

struct Gpu {
    node: DrmNode,
    manager: GbmDrmOutputManager,
    scanner: DrmScanner,
    renderer: GlesRenderer,
    surface: Option<Surface>,
    token: RegistrationToken,
}

struct Surface {
    crtc: crtc::Handle,
    connector: connector::Handle,
    /// The mode set on the CRTC (the connector's preferred one)
    mode: DrmMode,
    output: Output,
    drm_output: GbmDrmOutput,
    global: GlobalId,
    /// A frame was queued and its vblank has not arrived yet
    queued: bool,
}

impl DrmData {
    pub fn seat_name(&self) -> String {
        self.session.seat()
    }

    pub fn renderer(&mut self) -> Option<&mut GlesRenderer> {
        self.device.as_mut().map(|gpu| &mut gpu.renderer)
    }

    pub fn change_vt(&mut self, vt: i32) {
        if let Err(err) = self.session.change_vt(vt) {
            warn!("cannot switch to VT {vt}: {err}");
        }
    }

    pub fn update_led_state(&mut self, leds: LedState) {
        for keyboard in &mut self.keyboards {
            keyboard.led_update(leds.into());
        }
    }
}

pub fn run(options: &Options) -> Result<i32, Box<dyn std::error::Error>> {
    let mut event_loop: EventLoop<State> = EventLoop::try_new()?;
    let display: Display<State> = Display::new()?;

    let (session, notifier) = LibSeatSession::new().map_err(|err| format!("no session (libseat/logind): {err}"))?;
    let seat = session.seat();
    let gpu_path = primary_gpu(&seat)?.or_else(|| all_gpus(&seat).ok()?.into_iter().next()).ok_or("no GPU found")?;
    info!("seat {seat}, GPU {}", gpu_path.display());

    let udev = UdevBackend::new(&seat)?;
    let data =
        DrmData { session: session.clone(), gpu_path, device: None, keyboards: Vec::new(), render_scheduled: false };
    let mut state = State::new(display, event_loop.handle(), event_loop.get_signal(), Backend::Drm(Box::new(data)))?;

    for (device_id, path) in udev.device_list() {
        if state.drm().is_some_and(|d| d.device.is_none() && d.gpu_path == path)
            && let Ok(node) = DrmNode::from_dev_id(device_id)
        {
            state.drm_device_added(node, path)?;
        }
    }
    if state.drm().is_none_or(|d| d.device.is_none()) {
        return Err("cannot open the GPU".into());
    }
    state.drm_init_globals();

    let mut libinput = Libinput::new_with_udev::<LibinputSessionInterface<LibSeatSession>>(session.clone().into());
    libinput.udev_assign_seat(&seat).map_err(|()| "libinput cannot assign the seat")?;
    event_loop.handle().insert_source(LibinputInputBackend::new(libinput.clone()), |mut event, _, state| {
        if let Some(drm) = state.drm() {
            match &mut event {
                InputEvent::DeviceAdded { device } if device.has_capability(DeviceCapability::Keyboard) => {
                    drm.keyboards.push(device.clone());
                }
                InputEvent::DeviceRemoved { device } => drm.keyboards.retain(|k| k != device),
                _ => {}
            }
        }
        state.process_input(event);
    })?;

    event_loop.handle().insert_source(notifier, move |event, _, state| match event {
        SessionEvent::PauseSession => {
            info!("session paused (VT switch)");
            libinput.suspend();
            if let Some(gpu) = state.drm().and_then(|d| d.device.as_mut()) {
                gpu.manager.pause();
            }
        }
        SessionEvent::ActivateSession => {
            info!("session active again");
            if libinput.resume().is_err() {
                error!("cannot resume libinput");
            }
            if let Some(gpu) = state.drm().and_then(|d| d.device.as_mut()) {
                if let Err(err) = gpu.manager.lock().activate(false) {
                    error!("cannot re-activate the DRM device: {err}");
                }
                if let Some(surface) = gpu.surface.as_mut() {
                    surface.queued = false;
                }
            }
            state.schedule_drm_render(Duration::ZERO);
        }
    })?;

    event_loop.handle().insert_source(udev, |event, _, state| match event {
        UdevEvent::Added { device_id, path } => {
            if state.drm().is_some_and(|d| d.device.is_none() && d.gpu_path == path)
                && let Ok(node) = DrmNode::from_dev_id(device_id)
                && let Err(err) = state.drm_device_added(node, &path)
            {
                error!("cannot open {}: {err}", path.display());
            }
        }
        UdevEvent::Changed { device_id } => {
            if state.drm().and_then(|d| d.device.as_ref()).is_some_and(|gpu| gpu.node.dev_id() == device_id) {
                state.drm_device_changed();
            }
        }
        UdevEvent::Removed { device_id } => {
            if state.drm().and_then(|d| d.device.as_ref()).is_some_and(|gpu| gpu.node.dev_id() == device_id) {
                state.drm_device_removed();
            }
        }
    })?;

    state.start(options);

    while state.running && !crate::shutdown_requested() {
        if event_loop.dispatch(Some(Duration::from_millis(16)), &mut state).is_err() {
            state.running = false;
        }
        state.space.refresh();
        state.popups.cleanup();
        state.control.flush();
        let _ = state.display_handle.flush_clients();
    }
    Ok(state.shutdown())
}

impl State {
    fn drm(&mut self) -> Option<&mut DrmData> {
        match &mut self.backend {
            Backend::Drm(data) => Some(data),
            Backend::Winit(_) => None,
        }
    }

    fn drm_device_added(&mut self, node: DrmNode, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
        let dh = self.display_handle.clone();
        let Some(drm) = self.drm() else { return Ok(()) };
        let fd = drm.session.open(path, OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOCTTY | OFlags::NONBLOCK)?;
        let fd = DrmDeviceFd::new(DeviceFd::from(fd));
        let (device, notifier) = DrmDevice::new(fd.clone(), true)?;
        let gbm = GbmDevice::new(fd)?;

        // Safety: the GBM device outlives the display (both live in `Gpu`).
        let egl_display = unsafe { EGLDisplay::new(gbm.clone())? };
        // A software EGL device (llvmpipe via kms_swrast, e.g. virtio-vga without host GL in a VM)
        // has no render node: it renders on the primary node with linear buffers. anvil refuses
        // such devices; we accept them (slow, but enough to test the session).
        let egl_device = EGLDevice::device_for_display(&egl_display).ok();
        let software = egl_device.as_ref().is_some_and(EGLDevice::is_software);
        let render_node = egl_device.and_then(|device| device.try_get_render_node().ok().flatten()).unwrap_or(node);
        if software {
            warn!("{} only has software rendering (llvmpipe); expect low frame rates", path.display());
        }
        let context = EGLContext::new_with_priority(&egl_display, ContextPriority::High)?;
        // Safety: the context is only used by this renderer.
        let mut renderer = unsafe { GlesRenderer::new(context)? };
        if renderer.bind_wl_display(&dh).is_ok() {
            info!("EGL hardware acceleration (wl_drm) enabled");
        }

        let allocator = GbmAllocator::new(gbm.clone(), GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT);
        let exporter = GbmFramebufferExporter::new(gbm.clone(), Some(render_node).into());
        let render_formats: FormatSet = renderer
            .egl_context()
            .dmabuf_render_formats()
            .iter()
            .filter(|format| !software || format.modifier == Modifier::Linear)
            .copied()
            .collect();
        let manager = DrmOutputManager::new(
            device,
            allocator,
            exporter,
            Some(gbm),
            COLOR_FORMATS.iter().copied(),
            render_formats,
        );

        let token = self.loop_handle.insert_source(notifier, move |event, metadata, state| match event {
            DrmEvent::VBlank(crtc) => state.drm_frame_finish(crtc, metadata),
            DrmEvent::Error(err) => error!("DRM error: {err}"),
        })?;
        info!("opened {} (render node {render_node})", path.display());
        let Some(drm) = self.drm() else { return Ok(()) };
        drm.device = Some(Gpu { node, manager, scanner: DrmScanner::new(), renderer, surface: None, token });
        self.drm_device_changed();
        Ok(())
    }

    /// linux-dmabuf with the renderer's formats, and explicit sync if the GPU supports it.
    fn drm_init_globals(&mut self) {
        let dh = self.display_handle.clone();
        let Some(gpu) = self.drm().and_then(|d| d.device.as_mut()) else { return };
        let formats = gpu.renderer.dmabuf_formats();
        let shm_formats = gpu.renderer.shm_formats();
        let import_device = gpu.manager.device().device_fd().clone();
        let render_dev = gpu.node.node_with_type(NodeType::Render).and_then(Result::ok).unwrap_or(gpu.node).dev_id();
        self.shm_state.update_formats(shm_formats);
        match DmabufFeedbackBuilder::new(render_dev, formats.clone()).build() {
            Ok(feedback) => {
                self.dmabuf_global =
                    Some(self.dmabuf_state.create_global_with_default_feedback::<State>(&dh, &feedback));
            }
            Err(err) => {
                warn!("no dmabuf feedback ({err}), linux-dmabuf v3");
                self.dmabuf_global = Some(self.dmabuf_state.create_global::<State>(&dh, formats));
            }
        }
        if supports_syncobj_eventfd(&import_device) {
            self.syncobj_state = Some(DrmSyncobjState::new::<State>(&dh, import_device));
            info!("explicit sync (linux-drm-syncobj) enabled");
        }
    }

    /// Re-scans the connectors: drops our output when its connector is gone and takes the first
    /// connected one when we have none.
    fn drm_device_changed(&mut self) {
        let Some(gpu) = self.drm().and_then(|d| d.device.as_mut()) else { return };
        let events = match gpu.scanner.scan_connectors(gpu.manager.device()) {
            Ok(events) => events,
            Err(err) => {
                warn!("cannot scan connectors: {err}");
                return;
            }
        };
        let mut lost = false;
        let mut changed = None;
        for event in events {
            match event {
                DrmScanEvent::Disconnected { connector, .. }
                    if gpu.surface.as_ref().is_some_and(|s| s.connector == connector.handle()) =>
                {
                    info!("output {:?} unplugged", connector.handle());
                    lost = true;
                }
                // Same monitor, other modes (a TV switched its EDID, a VM window was resized)
                DrmScanEvent::Changed { connector, .. }
                    if gpu.surface.as_ref().is_some_and(|s| s.connector == connector.handle()) =>
                {
                    changed = Some(connector);
                }
                _ => {}
            }
        }
        if lost && let Some(surface) = gpu.surface.take() {
            self.space.unmap_output(&surface.output);
            self.display_handle.remove_global::<State>(surface.global);
            self.output = None;
        }
        if !lost && let Some(connector) = changed {
            self.drm_connector_modes_changed(&connector);
        }
        let Some(gpu) = self.drm().and_then(|d| d.device.as_mut()) else { return };
        if gpu.surface.is_none() {
            let candidate = gpu.scanner.crtcs().next().map(|(info, crtc)| (info.clone(), crtc));
            if let Some((connector, crtc)) = candidate {
                self.drm_connector_connected(connector, crtc);
            }
        }
    }

    /// Switches our output to the connector's new preferred mode, if it has another one.
    fn drm_connector_modes_changed(&mut self, connector: &connector::Info) {
        let Some(gpu) = self.drm().and_then(|d| d.device.as_mut()) else { return };
        let Some(surface) = gpu.surface.as_mut() else { return };
        let Some(mode) = preferred_mode(connector).filter(|mode| *mode != surface.mode) else { return };
        let wl_mode = WlMode::from(mode);
        if let Err(err) = surface.drm_output.use_mode::<_, OutputElement>(
            mode,
            &mut gpu.renderer,
            &DrmOutputRenderElements::default(),
        ) {
            warn!("cannot switch {} to {}x{}: {err}", surface.output.name(), wl_mode.size.w, wl_mode.size.h);
            return;
        }
        info!("output {}: mode {}x{}@{}", surface.output.name(), wl_mode.size.w, wl_mode.size.h, wl_mode.refresh);
        surface.mode = mode;
        let output = surface.output.clone();
        output.set_preferred(wl_mode);
        output.change_current_state(Some(wl_mode), None, None, None);
        // Clients and the shell learn the new size; windows are refitted.
        self.set_output(output);
        self.schedule_drm_render(Duration::ZERO);
    }

    fn drm_connector_connected(&mut self, connector: connector::Info, crtc: crtc::Handle) {
        let dh = self.display_handle.clone();
        let Some(gpu) = self.drm().and_then(|d| d.device.as_mut()) else { return };
        let name = format!("{}-{}", connector.interface().as_str(), connector.interface_id());
        let Some(drm_mode) = preferred_mode(&connector) else {
            warn!("connector {name} has no modes");
            return;
        };
        let device = gpu.manager.device();
        let info = display_info::for_connector(device, connector.handle());
        let (width_mm, height_mm) = connector.size().unwrap_or((0, 0));
        let output = Output::new(
            name.clone(),
            PhysicalProperties {
                size: (width_mm as i32, height_mm as i32).into(),
                subpixel: connector.subpixel().into(),
                make: info.as_ref().and_then(|i| i.make()).unwrap_or_else(|| "Unknown".into()),
                model: info.as_ref().and_then(|i| i.model()).unwrap_or_else(|| "Unknown".into()),
                serial_number: info.as_ref().and_then(|i| i.serial()).unwrap_or_else(|| "Unknown".into()),
            },
        );
        let wl_mode = WlMode::from(drm_mode);
        output.set_preferred(wl_mode);
        output.change_current_state(Some(wl_mode), Some(Transform::Normal), None, Some((0, 0).into()));

        let mut planes = match device.planes(&crtc) {
            Ok(planes) => planes,
            Err(err) => {
                warn!("cannot query planes of {name}: {err}");
                return;
            }
        };
        // Overlay planes break on NVIDIA (anvil does the same).
        if let Ok(driver) = device.get_driver()
            && (driver.name().to_string_lossy().to_lowercase().contains("nvidia")
                || driver.description().to_string_lossy().to_lowercase().contains("nvidia"))
        {
            planes.overlay = vec![];
        }

        let drm_output = match gpu.manager.lock().initialize_output::<_, OutputElement>(
            crtc,
            drm_mode,
            &[connector.handle()],
            &output,
            Some(planes),
            &mut gpu.renderer,
            &DrmOutputRenderElements::default(),
        ) {
            Ok(drm_output) => drm_output,
            Err(err) => {
                warn!("cannot initialize output {name}: {err}");
                return;
            }
        };
        let global = output.create_global::<State>(&dh);
        info!("output {name}: {}x{}@{}", wl_mode.size.w, wl_mode.size.h, wl_mode.refresh);
        gpu.surface = Some(Surface {
            crtc,
            connector: connector.handle(),
            mode: drm_mode,
            output: output.clone(),
            drm_output,
            global,
            queued: false,
        });
        self.set_output(output);
        self.schedule_drm_render(Duration::ZERO);
    }

    fn drm_device_removed(&mut self) {
        let Some(drm) = self.drm() else { return };
        let Some(gpu) = drm.device.take() else { return };
        if let Some(surface) = gpu.surface {
            self.space.unmap_output(&surface.output);
            self.display_handle.remove_global::<State>(surface.global);
        }
        self.loop_handle.remove(gpu.token);
        self.output = None;
        warn!("the GPU is gone");
    }

    fn frame_duration(output: &Output) -> Duration {
        output
            .current_mode()
            .map(|mode| Duration::from_secs_f64(1000.0 / mode.refresh.max(1) as f64))
            .unwrap_or(Duration::from_millis(16))
    }

    fn schedule_drm_render(&mut self, delay: Duration) {
        let Some(drm) = self.drm() else { return };
        if drm.render_scheduled {
            return;
        }
        drm.render_scheduled = true;
        let result = self.loop_handle.insert_source(Timer::from_duration(delay), |_, _, state| {
            if let Some(drm) = state.drm() {
                drm.render_scheduled = false;
            }
            state.render_drm();
            TimeoutAction::Drop
        });
        if let Err(err) = result {
            warn!("cannot schedule a frame: {err}");
        }
    }

    fn render_drm(&mut self) {
        let pointer = self.pointer.current_location();
        let Backend::Drm(drm) = &mut self.backend else { return };
        let Some(gpu) = drm.device.as_mut() else { return };
        let Some(surface) = gpu.surface.as_mut() else { return };
        if surface.queued || !drm.session.is_active() {
            return;
        }
        let output = surface.output.clone();
        let elements = render::output_elements(&mut gpu.renderer, &self.space, &output, &self.cursor_status, pointer);
        let result =
            surface.drm_output.render_frame(&mut gpu.renderer, &elements, render::CLEAR_COLOR, FrameFlags::DEFAULT);
        let mut rendered = false;
        match result {
            Ok(result) => {
                rendered = !result.is_empty;
                let states = result.states;
                render::update_primary_scanout_output(&self.space, &output, &states);
                if rendered {
                    let feedback = render::take_presentation_feedback(&output, &self.space, &states);
                    match surface.drm_output.queue_frame(Some(feedback)) {
                        Ok(()) => surface.queued = true,
                        Err(err) => {
                            warn!("cannot queue the frame: {err}");
                            rendered = false;
                        }
                    }
                }
            }
            Err(err) => warn!("rendering failed: {err}"),
        }
        drop(elements);
        render::send_frames(&self.space, &output, self.clock.now().into(), &self.cursor_status);
        self.take_screenshots(&output);
        if !rendered {
            // Nothing changed (or a temporary error): look again in one frame.
            self.schedule_drm_render(Self::frame_duration(&output));
        }
    }

    fn drm_frame_finish(&mut self, crtc: crtc::Handle, metadata: &mut Option<DrmEventMetadata>) {
        let now = self.clock.now();
        let Some(gpu) = self.drm().and_then(|d| d.device.as_mut()) else { return };
        let Some(surface) = gpu.surface.as_mut().filter(|s| s.crtc == crtc) else { return };
        surface.queued = false;
        let frame_duration = Self::frame_duration(&surface.output);
        let (time, flags) = match metadata.as_ref().map(|m| m.time) {
            Some(DrmEventTime::Monotonic(time)) if !time.is_zero() => (
                time.into(),
                wp_presentation_feedback::Kind::Vsync
                    | wp_presentation_feedback::Kind::HwClock
                    | wp_presentation_feedback::Kind::HwCompletion,
            ),
            _ => (now, wp_presentation_feedback::Kind::Vsync),
        };
        let sequence = metadata.as_ref().map(|m| m.sequence).unwrap_or(0);
        match surface.drm_output.frame_submitted() {
            Ok(user_data) => {
                if let Some(mut feedback) = user_data.flatten() {
                    feedback.presented(time, Refresh::fixed(frame_duration), sequence as u64, flags);
                }
            }
            Err(err) => warn!("frame submission failed: {err}"),
        }
        // Render a bit after the vblank, so clients have time for their next frame (anvil).
        self.schedule_drm_render(frame_duration.mul_f64(0.6));
    }
}

/// The connector's preferred mode, else its first one.
fn preferred_mode(connector: &connector::Info) -> Option<DrmMode> {
    connector
        .modes()
        .iter()
        .find(|mode| mode.mode_type().contains(ModeTypeFlags::PREFERRED))
        .or_else(|| connector.modes().first())
        .copied()
}
