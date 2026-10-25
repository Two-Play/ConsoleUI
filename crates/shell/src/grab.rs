//! Exclusive access to the controllers while the control center is open: `EVIOCGRAB` locks the
//! evdev device for all other readers (the game, but also gilrs). The shell then reads the events
//! itself. Games that read the controller via hidraw (e.g. Steam Input) are not affected.

use std::fs::{self, File, OpenOptions};
use std::io::Read;
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::time::Instant;

use consoleui_core::Action;
use consoleui_core::input::evdev::{self, ABS_HAT0X, ABS_HAT0Y, ABS_X, ABS_Y, EV_ABS, EV_KEY};
use consoleui_core::input::{DirectionRepeat, Stick, StickChange, is_direction};

/// `_IOW('E', 0x90, int)`
const EVIOCGRAB: libc::c_ulong = 0x4004_4590;
/// `_IOR('E', 0x40 + abs, struct input_absinfo)`
const EVIOCGABS_BASE: libc::c_ulong = 0x8018_4540;
/// `struct input_event` on 64-bit: timeval (16) + type (2) + code (2) + value (4)
const EVENT_SIZE: usize = 24;

#[repr(C)]
#[derive(Default)]
struct AbsInfo {
    value: i32,
    minimum: i32,
    maximum: i32,
    fuzz: i32,
    flat: i32,
    resolution: i32,
}

struct Device {
    file: File,
    /// (min, max) for ABS_X and ABS_Y
    range: [(i32, i32); 2],
    axes: [f32; 2],
    hat: [f32; 2],
    stick: Stick,
    hat_stick: Stick,
}

pub struct Grab {
    devices: Vec<Device>,
    repeat: DirectionRepeat,
}

impl Grab {
    /// Grabs all evdev devices with one of the names (as gilrs reports them).
    pub fn acquire(names: &[String]) -> Self {
        let devices = fs::read_dir("/sys/class/input")
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|entry| {
                let node = entry.file_name().into_string().ok()?;
                if !node.starts_with("event") {
                    return None;
                }
                let name = fs::read_to_string(entry.path().join("device/name")).ok()?;
                names.iter().any(|n| n == name.trim()).then(|| format!("/dev/input/{node}"))
            })
            .filter_map(|path| open(&path).map_err(|err| eprintln!("Controller {path} not exclusive: {err}")).ok())
            .collect();
        Self { devices, repeat: DirectionRepeat::default() }
    }

    pub fn is_empty(&self) -> bool {
        self.devices.is_empty()
    }

    pub fn poll(&mut self, now: Instant) -> Vec<Action> {
        let mut actions = Vec::new();
        let mut buffer = [0u8; EVENT_SIZE * 32];
        for device in &mut self.devices {
            // Non-blocking: returns `WouldBlock` as soon as nothing is pending.
            while let Ok(read) = device.file.read(&mut buffer) {
                if read == 0 {
                    break;
                }
                for event in buffer[..read].as_chunks::<EVENT_SIZE>().0 {
                    let kind = u16::from_ne_bytes([event[16], event[17]]);
                    let code = u16::from_ne_bytes([event[18], event[19]]);
                    let value = i32::from_ne_bytes([event[20], event[21], event[22], event[23]]);
                    handle(device, &mut self.repeat, kind, code, value, now, &mut actions);
                }
            }
        }
        actions.extend(self.repeat.poll(now));
        actions
    }
}

fn handle(
    device: &mut Device,
    repeat: &mut DirectionRepeat,
    kind: u16,
    code: u16,
    value: i32,
    now: Instant,
    actions: &mut Vec<Action>,
) {
    let apply = |change: StickChange, repeat: &mut DirectionRepeat, actions: &mut Vec<Action>| {
        if let Some(released) = change.released {
            repeat.release(released);
        }
        if let Some(pressed) = change.pressed {
            actions.push(repeat.press(pressed, now));
        }
    };
    match (kind, code) {
        (EV_KEY, _) => {
            let Some(action) = evdev::key_action(code) else { return };
            match (value, is_direction(action)) {
                (1, true) => actions.push(repeat.press(action, now)),
                (1, false) => actions.push(action),
                (0, true) => repeat.release(action),
                _ => {}
            }
        }
        (EV_ABS, ABS_X | ABS_Y) => {
            let axis = usize::from(code == ABS_Y);
            let (min, max) = device.range[axis];
            device.axes[axis] = evdev::normalize(value, min, max);
            // evdev counts downwards as positive, `Stick` expects "up positive".
            let change = device.stick.update(device.axes[0], -device.axes[1]);
            apply(change, repeat, actions);
        }
        (EV_ABS, ABS_HAT0X | ABS_HAT0Y) => {
            device.hat[usize::from(code == ABS_HAT0Y)] = value.signum() as f32;
            let change = device.hat_stick.update(device.hat[0], -device.hat[1]);
            apply(change, repeat, actions);
        }
        _ => {}
    }
}

fn open(path: &str) -> std::io::Result<Device> {
    let file = OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK).open(path)?;
    let fd = file.as_raw_fd();
    // SAFETY: valid descriptor, EVIOCGRAB expects an int value (1 = grab).
    if unsafe { libc::ioctl(fd, EVIOCGRAB, 1 as libc::c_int) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    let range = [ABS_X, ABS_Y].map(|axis| {
        let mut info = AbsInfo::default();
        // SAFETY: `info` is a writable `input_absinfo` of the right size.
        let ok = unsafe { libc::ioctl(fd, EVIOCGABS_BASE + libc::c_ulong::from(axis), &mut info) } == 0;
        if ok { (info.minimum, info.maximum) } else { (0, 0) }
    });
    Ok(Device { file, range, axes: [0.0; 2], hat: [0.0; 2], stick: Stick::default(), hat_stick: Stick::default() })
}
