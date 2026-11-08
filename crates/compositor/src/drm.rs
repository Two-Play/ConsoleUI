//! Session backend: DRM/KMS on the primary GPU, libinput for keyboard and mouse, libseat (logind)
//! for device access and VT switching. Single GPU, single output (see `pick_output`; another one
//! takes over when it is unplugged, and a preferred one when it is plugged in).
//!
//! Modelled on anvil's `udev.rs` (MIT, Copyright (c) 2017 Victor Berger), without multi-GPU,
//! DRM leasing and the debug overlays.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use smithay::backend::allocator::format::FormatSet;
use smithay::backend::allocator::gbm::{GbmAllocator, GbmBufferFlags, GbmDevice};
use smithay::backend::allocator::{Fourcc, Modifier};
use smithay::backend::drm::compositor::FrameFlags;
use smithay::backend::drm::exporter::gbm::GbmFramebufferExporter;
use smithay::backend::drm::output::{DrmOutput, DrmOutputManager, DrmOutputRenderElements};
use smithay::backend::drm::{
    DrmDevice, DrmDeviceFd, DrmEvent, DrmEventMetadata, DrmEventTime, DrmNode, NodeType, Planes, VrrSupport,
};
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
use smithay::reexports::drm::control::{Mode as DrmMode, ModeFlags, ModeTypeFlags, connector, crtc};
use smithay::reexports::input::{DeviceCapability, Libinput};
use smithay::reexports::rustix::fs::OFlags;
use smithay::reexports::wayland_protocols::wp::linux_dmabuf::zv1::server::zwp_linux_dmabuf_feedback_v1::TrancheFlags;
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

use consoleui_core::protocol::{CompositorMessage, DisplayInfo};

use crate::backend::Backend;
use crate::modes::{self, ModeInfo};
use crate::options::Options;
use crate::render::{self, Drawn, OutputElement, ScanoutFeedback};
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
    /// The scheduled render (avoids piling up timers; a commit under VRR replaces it)
    render_timer: Option<RegistrationToken>,
    /// Size from the "Resolution" setting; `None` is the monitor's preferred mode (Story 5.5)
    wanted_size: Option<(u32, u32)>,
    /// Client buffers may go straight to a plane (Story 7.8; `--no-scanout` turns it off)
    scanout: bool,
    /// The "Variable refresh rate" setting of the shell (Story 7.9)
    vrr_wanted: bool,
    /// Plymouth's boot splash was on screen when we started (Story 6.11): it stays there until a
    /// window shows content or this deadline passes, instead of a black screen in between
    splash_until: Option<Instant>,
}

struct Gpu {
    node: DrmNode,
    manager: GbmDrmOutputManager,
    scanner: DrmScanner,
    renderer: GlesRenderer,
    surface: Option<Surface>,
    /// An output whose DRM side waits for the first frame worth showing (boot splash, Story 6.11)
    pending: Option<PendingOutput>,
    token: RegistrationToken,
}

/// Clients already see the output (its global exists), but the CRTC is not ours yet: Smithay
/// shows a frame as soon as the DRM output is set up, which would blank the boot splash. It is
/// set up with the shell's first frame instead (`State::finish_drm_output`).
struct PendingOutput {
    crtc: crtc::Handle,
    connector: connector::Info,
    mode: DrmMode,
    output: Output,
    global: GlobalId,
    planes: Planes,
    feedback: Option<ScanoutFeedback>,
}

struct Surface {
    crtc: crtc::Handle,
    connector: connector::Handle,
    /// The mode set on the CRTC
    mode: DrmMode,
    /// All modes of the connector
    modes: Vec<DrmMode>,
    output: Output,
    drm_output: GbmDrmOutput,
    global: GlobalId,
    /// A frame was queued and its vblank has not arrived yet
    queued: bool,
    /// What clients are told to allocate, by whether their buffers can reach a plane (Story 7.8)
    feedback: Option<ScanoutFeedback>,
    /// A window's buffer went straight to a plane in the last frame (only logged)
    zero_copy: bool,
    /// The monitor and driver can switch VRR without a modeset (Story 7.9)
    vrr_supported: bool,
    /// VRR is on right now: a game is in front and the setting allows it
    vrr_active: bool,
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
    let data = DrmData {
        session: session.clone(),
        gpu_path,
        device: None,
        keyboards: Vec::new(),
        render_timer: None,
        wanted_size: None,
        scanout: options.scanout,
        vrr_wanted: false,
        splash_until: boot_splash_running().then(|| Instant::now() + SPLASH_HOLD),
    };
    let mut state = State::new(display, event_loop.handle(), event_loop.get_signal(), Backend::Drm(Box::new(data)))?;
    state.preferred_output = options.output.clone();
    state.cli_output = options.output.clone();

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
                    // Whoever had the VT may have changed it
                    surface.vrr_active = surface.drm_output.with_compositor(|compositor| compositor.vrr_enabled());
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
        // Turning all connectors off first would blank the boot splash until our first frame
        let keep_splash = drm.splash_until.is_some();
        let (device, notifier) = DrmDevice::new(fd.clone(), !keep_splash)?;
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
        drm.device =
            Some(Gpu { node, manager, scanner: DrmScanner::new(), renderer, surface: None, pending: None, token });
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
        let mut plugged = false;
        for event in events {
            match event {
                DrmScanEvent::Connected { .. } => plugged = true,
                DrmScanEvent::Disconnected { connector, .. }
                    if gpu.surface.as_ref().is_some_and(|s| s.connector == connector.handle())
                        || gpu.pending.as_ref().is_some_and(|p| p.connector.handle() == connector.handle()) =>
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
        if lost {
            self.drop_drm_surface();
        }
        if !lost && let Some(connector) = changed {
            self.drm_connector_modes_changed(&connector);
        }
        self.drm_choose_output(plugged);
        self.announce_displays();
    }

    /// The "Screen" row of the shell (Story 7.11): this monitor from now on, also when it is
    /// plugged in later; `None` goes back to `--output` and the automatic choice.
    pub fn set_display(&mut self, name: Option<String>) {
        let preferred = name.or_else(|| self.cli_output.clone());
        if preferred == self.preferred_output {
            return;
        }
        info!("preferred output: {}", preferred.as_deref().unwrap_or("automatic"));
        self.preferred_output = preferred;
        self.drm_choose_output(true);
        self.announce_displays();
    }

    /// Takes the monitor `pick_output` wants. One in use stays unless `switch` (a monitor was
    /// plugged in, or the choice changed).
    fn drm_choose_output(&mut self, switch: bool) {
        let preferred = self.preferred_output.clone();
        let Some(gpu) = self.drm().and_then(|d| d.device.as_mut()) else { return };
        let candidates: Vec<_> = gpu.scanner.crtcs().map(|(info, crtc)| (info.clone(), crtc)).collect();
        let names: Vec<String> = candidates.iter().map(|(info, _)| connector_name(info)).collect();
        let Some((connector, crtc)) =
            pick_output(&names, preferred.as_deref()).and_then(|index| candidates.into_iter().nth(index))
        else {
            return;
        };
        // A monitor we prefer was plugged in or chosen while another one is in use (Story 7.11)
        let current = gpu
            .surface
            .as_ref()
            .map(|surface| surface.connector)
            .or_else(|| gpu.pending.as_ref().map(|pending| pending.connector.handle()));
        if switch && current.is_some_and(|current| current != connector.handle()) {
            info!("switching to the output {}", connector_name(&connector));
            self.drop_drm_surface();
        } else if current.is_some() {
            return;
        }
        self.drm_connector_connected(connector, crtc);
    }

    /// Gives up our output; dropping the DRM output turns its CRTC off.
    fn drop_drm_surface(&mut self) {
        let Some(gpu) = self.drm().and_then(|d| d.device.as_mut()) else { return };
        let gone = match (gpu.surface.take(), gpu.pending.take()) {
            (Some(surface), _) => (surface.output, surface.global),
            (None, Some(pending)) => (pending.output, pending.global),
            (None, None) => return,
        };
        self.space.unmap_output(&gone.0);
        self.display_handle.remove_global::<State>(gone.1);
        self.output = None;
    }

    /// The connector has other modes now: the shell learns them, and the output follows.
    fn drm_connector_modes_changed(&mut self, connector: &connector::Info) {
        let Some(data) = self.drm() else { return };
        let wanted = data.wanted_size;
        let Some(surface) = data.device.as_mut().and_then(|gpu| gpu.surface.as_mut()) else { return };
        surface.modes = connector.modes().to_vec();
        if let Some(mode) = pick_mode(connector.modes(), wanted) {
            self.switch_mode(mode);
        }
        self.announce_modes();
    }

    /// The "Resolution" setting of the shell (Story 5.5); also holds for monitors plugged in later.
    /// Nested there are no modes.
    pub fn set_output_resolution(&mut self, size: Option<(u32, u32)>) {
        let Some(data) = self.drm() else { return };
        data.wanted_size = size;
        let Some(surface) = data.device.as_ref().and_then(|gpu| gpu.surface.as_ref()) else { return };
        if let Some(mode) = pick_mode(&surface.modes, size) {
            self.switch_mode(mode);
        }
    }

    /// The "Variable refresh rate" setting of the shell (Story 7.9). It takes effect with the next
    /// frame that shows a game, see [`Self::update_vrr`].
    pub fn set_vrr(&mut self, enabled: bool) {
        let Some(data) = self.drm() else { return };
        data.vrr_wanted = enabled;
        self.schedule_drm_render(Duration::ZERO);
    }

    /// VRR is on only while a game is in front: the shell's animations and overlays run at the
    /// monitor's fixed rate, and some panels flicker with them otherwise. Changing it needs no
    /// modeset (`vrr_supported`), the next commit carries it.
    fn update_vrr(&mut self) {
        let wanted = self.policy.wants_vrr();
        let Backend::Drm(drm) = &mut self.backend else { return };
        // On another VT the CRTC is not ours: a failure here would say nothing about the monitor.
        if !drm.session.is_active() {
            return;
        }
        let wanted = wanted && drm.vrr_wanted;
        let Some(surface) = drm.device.as_mut().and_then(|gpu| gpu.surface.as_mut()) else { return };
        if !surface.vrr_supported || surface.vrr_active == wanted {
            return;
        }
        match surface.drm_output.with_compositor(|compositor| compositor.use_vrr(wanted)) {
            Ok(()) => {
                info!("output {}: VRR {}", surface.output.name(), if wanted { "on" } else { "off" });
                surface.vrr_active = wanted;
            }
            Err(err) => {
                warn!("cannot change VRR on {}: {err}", surface.output.name());
                surface.vrr_supported = false;
            }
        }
    }

    /// Under VRR the monitor waits for us, so a commit of the game goes out at once instead of at
    /// the next timer (Story 7.9).
    pub fn drm_surface_committed(&mut self) {
        let Some(data) = self.drm() else { return };
        let active =
            data.device.as_ref().and_then(|gpu| gpu.surface.as_ref()).is_some_and(|s| s.vrr_active && !s.queued);
        if !active {
            return;
        }
        if let Some(token) = data.render_timer.take() {
            self.loop_handle.remove(token);
        }
        self.schedule_drm_render(Duration::ZERO);
    }

    /// Tells the shell which monitors are connected, for its "Screen" row.
    fn announce_displays(&mut self) {
        let Some(gpu) = self.drm().and_then(|d| d.device.as_ref()) else { return };
        let current = gpu
            .surface
            .as_ref()
            .map(|surface| surface.connector)
            .or_else(|| gpu.pending.as_ref().map(|pending| pending.connector.handle()));
        let device = gpu.manager.device();
        let displays = gpu
            .scanner
            .crtcs()
            .map(|(info, _)| {
                let edid = display_info::for_connector(device, info.handle());
                let parts = edid.map(|edid| [edid.make(), edid.model()]).unwrap_or_default();
                let description = parts.into_iter().flatten().collect::<Vec<_>>().join(" ");
                DisplayInfo { name: connector_name(info), description, active: Some(info.handle()) == current }
            })
            .collect();
        let message = CompositorMessage::Displays { displays };
        self.control.set_greeting(message.clone());
        self.control.broadcast(&message);
    }

    fn announce_modes(&mut self) {
        let Some(surface) = self.drm().and_then(|d| d.device.as_ref()).and_then(|gpu| gpu.surface.as_ref()) else {
            return;
        };
        let message =
            CompositorMessage::Modes { sizes: modes::sizes(&mode_infos(&surface.modes)), vrr: surface.vrr_supported };
        self.control.set_greeting(message.clone());
        self.control.broadcast(&message);
    }

    /// Switches our output to `mode`, if it is not the current one.
    fn switch_mode(&mut self, mode: DrmMode) {
        let Some(gpu) = self.drm().and_then(|d| d.device.as_mut()) else { return };
        let Some(surface) = gpu.surface.as_mut().filter(|surface| surface.mode != mode) else { return };
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
        let Some(data) = self.drm() else { return };
        let wanted = data.wanted_size;
        let Some(gpu) = data.device.as_mut() else { return };
        let name = connector_name(&connector);
        let Some(drm_mode) = pick_mode(connector.modes(), wanted) else {
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

        let feedback = scanout_feedback(gpu.node, &gpu.renderer, &planes);
        let global = output.create_global::<State>(&dh);
        info!("output {name}: {}x{}@{}", wl_mode.size.w, wl_mode.size.h, wl_mode.refresh);
        gpu.pending =
            Some(PendingOutput { crtc, connector, mode: drm_mode, output: output.clone(), global, planes, feedback });
        let hold_splash = data.splash_until.is_some();
        self.set_output(output);
        if hold_splash {
            // render_drm finishes it once a window shows content
            self.schedule_drm_render(Duration::ZERO);
        } else {
            self.finish_drm_output(None);
        }
    }

    /// Sets up the DRM side of the pending output. Smithay shows a first frame right away: the
    /// given windows, or only the clear color.
    fn finish_drm_output(&mut self, windows: Option<Vec<Drawn>>) {
        let pointer = self.pointer.current_location();
        let Backend::Drm(drm) = &mut self.backend else { return };
        let Some(gpu) = drm.device.as_mut() else { return };
        let Some(pending) = gpu.pending.take() else { return };
        let PendingOutput { crtc, connector, mode, output, global, planes, feedback } = pending;
        let name = output.name();
        let mut first_frame = DrmOutputRenderElements::default();
        if let Some(windows) = windows {
            let elements = render::output_elements(
                &mut gpu.renderer,
                &windows,
                &output,
                &self.cursor_status,
                pointer,
                &mut self.blur_cache,
            );
            first_frame.add_output(&crtc, render::CLEAR_COLOR.into(), elements);
        }
        let drm_output = match gpu.manager.lock().initialize_output::<_, OutputElement>(
            crtc,
            mode,
            &[connector.handle()],
            &output,
            Some(planes),
            &mut gpu.renderer,
            &first_frame,
        ) {
            Ok(drm_output) => drm_output,
            Err(err) => {
                warn!("cannot initialize output {name}: {err}");
                self.space.unmap_output(&output);
                self.display_handle.remove_global::<State>(global);
                self.output = None;
                return;
            }
        };
        let vrr = drm_output.with_compositor(|compositor| compositor.vrr_supported(connector.handle()));
        let vrr_supported = matches!(vrr, Ok(VrrSupport::Supported));
        if matches!(vrr, Ok(VrrSupport::RequiresModeset)) {
            info!("output {name}: VRR would need a modeset, not used");
        }
        gpu.surface = Some(Surface {
            crtc,
            connector: connector.handle(),
            mode,
            modes: connector.modes().to_vec(),
            output,
            drm_output,
            global,
            queued: false,
            feedback,
            zero_copy: false,
            vrr_supported,
            vrr_active: false,
        });
        self.announce_modes();
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
        if self.drm().is_none_or(|drm| drm.render_timer.is_some()) {
            return;
        }
        let result = self.loop_handle.insert_source(Timer::from_duration(delay), |_, _, state| {
            if let Some(drm) = state.drm() {
                drm.render_timer = None;
            }
            state.render_drm();
            TimeoutAction::Drop
        });
        match result {
            Ok(token) => {
                if let Some(drm) = self.drm() {
                    drm.render_timer = Some(token);
                }
            }
            Err(err) => warn!("cannot schedule a frame: {err}"),
        }
    }

    fn render_drm(&mut self) {
        // Leave the boot splash on screen until there is something to show
        let content = self.policy.any_content();
        let hold_splash = match self.drm() {
            Some(drm) => match drm.splash_until {
                Some(until) if !content && Instant::now() < until => true,
                Some(_) => {
                    drm.splash_until = None;
                    info!("taking the screen over from the boot splash");
                    false
                }
                None => false,
            },
            None => false,
        };
        if hold_splash {
            self.schedule_drm_render(Duration::from_millis(16));
            return;
        }
        if self.drm().and_then(|drm| drm.device.as_ref()).is_some_and(|gpu| gpu.pending.is_some()) {
            let windows = content.then(|| self.frame_windows());
            self.finish_drm_output(windows);
            return;
        }
        self.update_vrr();
        let pointer = self.pointer.current_location();
        let windows = self.frame_windows();
        let Backend::Drm(drm) = &mut self.backend else { return };
        let Some(gpu) = drm.device.as_mut() else { return };
        let Some(surface) = gpu.surface.as_mut() else { return };
        if surface.queued || !drm.session.is_active() {
            return;
        }
        let output = surface.output.clone();
        let elements = render::output_elements(
            &mut gpu.renderer,
            &windows,
            &output,
            &self.cursor_status,
            pointer,
            &mut self.blur_cache,
        );
        let flags = if drm.scanout { FrameFlags::DEFAULT } else { FrameFlags::empty() };
        let result = surface.drm_output.render_frame(&mut gpu.renderer, &elements, render::CLEAR_COLOR, flags);
        let mut rendered = false;
        match result {
            Ok(result) => {
                rendered = !result.is_empty;
                let states = result.states;
                render::update_primary_scanout_output(&self.space, &output, &states);
                if let Some(feedback) = &surface.feedback {
                    render::send_dmabuf_feedback(&self.space, &output, &states, feedback);
                }
                if rendered {
                    let zero_copy = render::any_zero_copy(&windows, &states);
                    if zero_copy != surface.zero_copy {
                        info!("direct scanout {}", if zero_copy { "on" } else { "off" });
                        surface.zero_copy = zero_copy;
                    }
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
        let vrr = surface.vrr_active;
        let refresh = if vrr { Refresh::Variable(frame_duration) } else { Refresh::fixed(frame_duration) };
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
                    feedback.presented(time, refresh, sequence as u64, flags);
                }
            }
            Err(err) => warn!("frame submission failed: {err}"),
        }
        // Render a bit after the vblank, so clients have time for their next frame (anvil). Under
        // VRR there is no next vblank to meet, a frame that is already waiting goes out at once.
        self.schedule_drm_render(if vrr { Duration::ZERO } else { frame_duration.mul_f64(0.6) });
    }
}

/// What clients should allocate on this output (Story 7.8). A surface whose buffer can reach a
/// plane is offered the planes' formats first, so it can be shown without a copy. Limited to formats
/// we can sample too: an open overlay needs the buffer as a texture.
fn scanout_feedback(node: DrmNode, renderer: &GlesRenderer, planes: &Planes) -> Option<ScanoutFeedback> {
    let formats = renderer.dmabuf_formats();
    let render_dev = node.node_with_type(NodeType::Render).and_then(Result::ok).unwrap_or(node).dev_id();
    let plane_formats: FormatSet = planes
        .primary
        .iter()
        .chain(&planes.overlay)
        .flat_map(|plane| plane.formats.iter().copied())
        .filter(|format| formats.contains(format))
        .collect();
    let builder = DmabufFeedbackBuilder::new(render_dev, formats);
    let result = builder.clone().build().and_then(|render| {
        let scanout = builder.add_preference_tranche(node.dev_id(), TrancheFlags::Scanout, plane_formats, 4u32..=6);
        Ok(ScanoutFeedback { render, scanout: scanout.build()? })
    });
    result.map_err(|err| warn!("no scanout feedback: {err}")).ok()
}

/// The connector's name as the kernel and `--output` spell it, e.g. `HDMI-A-1`.
/// How long the boot splash may stay on screen while no window shows content
const SPLASH_HOLD: Duration = Duration::from_secs(5);

/// Whether Plymouth runs, i.e. its boot splash is on screen (the session's display manager
/// deactivated it, see session/install-plymouth.sh, and it quits once we draw).
fn boot_splash_running() -> bool {
    Path::new("/run/plymouth/pid").exists()
}

fn connector_name(connector: &connector::Info) -> String {
    format!("{}-{}", connector.interface().as_str(), connector.interface_id())
}

/// Which of the connected monitors to use (Story 7.11): the one named by `--output`, else an
/// external monitor (a console sits at a TV or monitor), else the first, which is the built-in
/// panel of a laptop.
fn pick_output(names: &[String], preferred: Option<&str>) -> Option<usize> {
    let internal = |name: &String| ["eDP", "LVDS", "DSI"].iter().any(|prefix| name.starts_with(prefix));
    preferred
        .and_then(|wanted| names.iter().position(|name| name.eq_ignore_ascii_case(wanted)))
        .or_else(|| names.iter().position(|name| !internal(name)))
        .or(if names.is_empty() { None } else { Some(0) })
}

fn mode_infos(modes: &[DrmMode]) -> Vec<ModeInfo> {
    modes
        .iter()
        .map(|mode| ModeInfo {
            size: (mode.size().0 as u32, mode.size().1 as u32),
            refresh: mode.vrefresh(),
            preferred: mode.mode_type().contains(ModeTypeFlags::PREFERRED),
            interlaced: mode.flags().contains(ModeFlags::INTERLACE),
        })
        .collect()
}

fn pick_mode(modes: &[DrmMode], wanted: Option<(u32, u32)>) -> Option<DrmMode> {
    modes::pick(&mode_infos(modes), wanted).map(|index| modes[index])
}

#[cfg(test)]
mod tests {
    use super::pick_output;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn prefers_the_named_then_an_external_output() {
        let all = names(&["eDP-1", "HDMI-A-1", "DP-2"]);
        assert_eq!(pick_output(&all, Some("dp-2")), Some(2));
        assert_eq!(pick_output(&all, Some("DP-9")), Some(1));
        assert_eq!(pick_output(&all, None), Some(1));
    }

    #[test]
    fn falls_back_to_the_internal_panel_or_nothing() {
        assert_eq!(pick_output(&names(&["eDP-1"]), None), Some(0));
        assert_eq!(pick_output(&[], Some("DP-1")), None);
    }
}
