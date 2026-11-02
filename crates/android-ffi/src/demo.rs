//! Thread-confined live original homebrew, with value-only synchronous callbacks.
use crate::draws::DrawOp;
use crate::graphics::ClearOp;
use emulator_gpu::{GraphicsBackend, GraphicsFrame};
use emulator_memory::Permissions;
use emulator_session::{RunOutcome, Session, LOAD_BASE};
use std::{cell::RefCell, rc::Rc};
const IMAGE: &[u8] = include_bytes!("../../../examples/homebrew/catch.nro");
const MAILBOX: u64 = LOAD_BASE + 0xa000;
#[repr(C)]
pub struct LiveDraw {
    pub abi_version: u32,
    pub status: u32,
    pub draw_count: u32,
    pub image_bytes: u32,
    pub background: ClearOp,
    pub draws: [DrawOp; 3],
    pub pixels: [u8; 16384],
}
#[repr(C)]
#[derive(Debug)]
pub struct DemoStatus {
    pub abi_version: u32,
    pub status: u32,
    pub frame: u32,
    pub score: u32,
    pub misses: u32,
    pub ball_x: u32,
    pub ball_y: u32,
    pub paddle_x: u32,
    pub instructions_low: u32,
    pub instructions_high: u32,
    pub error: [u8; 256],
}
impl Default for DemoStatus {
    fn default() -> Self {
        Self {
            abi_version: 1,
            status: 0,
            frame: 0,
            score: 0,
            misses: 0,
            ball_x: 0,
            ball_y: 0,
            paddle_x: 0,
            instructions_low: 0,
            instructions_high: 0,
            error: [0; 256],
        }
    }
}
#[derive(Debug)]
struct Callback(extern "C" fn(LiveDraw) -> u32);
impl GraphicsBackend for Callback {
    fn render(&self, frame: &GraphicsFrame) -> Result<(), String> {
        let c = frame.background;
        if c.width != 64 || c.height != 64 || frame.pixels.len() != 16384 || frame.draws.len() != 2
        {
            return Err("Live demo ABI requires 64x64/two draws".into());
        }
        let mut value = LiveDraw {
            abi_version: 1,
            status: 0,
            draw_count: 2,
            image_bytes: 16384,
            background: c.into(),
            draws: [DrawOp::default(); 3],
            pixels: [0; 16384],
        };
        for (out, draw) in value.draws.iter_mut().zip(&frame.draws) {
            let s = draw.surface;
            if draw.vertices.len() != 6
                || draw.texture.is_some()
                || draw.vertex.varying_outputs().is_some()
                || draw.vertex_spirv.len() > 2048
                || draw.fragment_spirv.len() > 2048
            {
                return Err("Live demo shader/vertex bounds".into());
            }
            out.address_upper = (s.address.0 >> 32) as u32;
            out.address_lower = s.address.0 as u32;
            out.width = s.width;
            out.height = s.height;
            out.pitch = s.pitch;
            out.format = s.format;
            out.rect = s.rect;
            out.mask = s.mask;
            out.vertex_count = 6;
            out.vertex_words = draw.vertex_spirv.len() as u32;
            out.fragment_words = draw.fragment_spirv.len() as u32;
            out.vertex[..draw.vertex_spirv.len()].copy_from_slice(&draw.vertex_spirv);
            out.fragment[..draw.fragment_spirv.len()].copy_from_slice(&draw.fragment_spirv);
            for (v, input) in out.vertices.chunks_exact_mut(4).zip(&draw.vertices) {
                v.copy_from_slice(input);
            }
        }
        value.pixels.copy_from_slice(&frame.pixels);
        let status = (self.0)(value);
        if status == 0 {
            Ok(())
        } else {
            Err(format!("Host rendering/completion failed: {status}"))
        }
    }
}
struct DemoSession {
    session: Session,
}
impl DemoSession {
    fn new(callback: extern "C" fn(LiveDraw) -> u32) -> Result<Self, String> {
        let session = Session::from_nro_with_graphics(IMAGE, Rc::new(Callback(callback)))
            .map_err(|e| e.to_string())?;
        Ok(Self { session })
    }
    fn word(&self, offset: usize) -> u32 {
        self.session
            .memory()
            .read_u32(MAILBOX + offset as u64)
            .expect("owned mailbox")
    }
    fn write(&self, offset: usize, value: u32) {
        self.session
            .memory()
            .pin(MAILBOX + offset as u64, 4, Permissions::WRITE)
            .expect("owned mailbox")
            .write(0, &value.to_le_bytes())
            .expect("owned mailbox pin");
    }
    fn status(&self) -> DemoStatus {
        DemoStatus {
            frame: self.word(16),
            score: self.word(20),
            misses: self.word(24),
            ball_x: self.word(28),
            ball_y: self.word(32),
            paddle_x: self.word(36),
            instructions_low: self.session.instructions() as u32,
            instructions_high: (self.session.instructions() >> 32) as u32,
            ..DemoStatus::default()
        }
    }
    fn step(&mut self, x: u32, reset: u32) -> Result<DemoStatus, String> {
        if x > 48 || reset > 1 {
            return Err("Invalid demo input".into());
        }
        self.write(4, x);
        self.write(8, reset);
        self.write(0, 0);
        for _ in 0..20000 {
            if self.session.run(1).map_err(|e| e.to_string())? != RunOutcome::BudgetExhausted {
                return Err("Guest stopped before frame handoff".into());
            }
            if self.word(0) == 1 {
                self.session.clear_ipc_trace();
                return Ok(self.status());
            }
        }
        Err("Guest frame instruction budget exceeded".into())
    }
    fn stop(mut self) -> Result<DemoStatus, String> {
        self.write(12, 1);
        self.write(0, 0);
        if self.session.run(20000).map_err(|e| e.to_string())? != RunOutcome::Exited
            || self.session.kernel().live_handle_count() != 0
            || self.session.kernel().live_transfer_memory_count() != 0
            || self.session.memory().lent_bytes() != 0
        {
            return Err("Guest shutdown resource mismatch".into());
        }
        Ok(self.status())
    }
}
thread_local! { static DEMO: RefCell<Option<DemoSession>> = const { RefCell::new(None) }; }
fn guarded(f: impl FnOnce(&mut Option<DemoSession>) -> Result<DemoStatus, String>) -> DemoStatus {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        DEMO.with(|cell| {
            let mut state = cell
                .try_borrow_mut()
                .map_err(|_| "Demo callback reentry".to_string())?;
            f(&mut state)
        })
    }))
    .unwrap_or_else(|_| Err("Live demo panic".into()));
    result.unwrap_or_else(|error| {
        let mut s = DemoStatus {
            status: 1,
            ..DemoStatus::default()
        };
        let len = error.len().min(255);
        s.error[..len].copy_from_slice(&error.as_bytes()[..len]);
        s
    })
}
#[no_mangle]
pub extern "C" fn emulator_demo_start_v1(callback: extern "C" fn(LiveDraw) -> u32) -> DemoStatus {
    guarded(|state| {
        if state.is_some() {
            return Err("Demo already running on this thread".into());
        }
        *state = Some(DemoSession::new(callback)?);
        Ok(DemoStatus::default())
    })
}
#[no_mangle]
pub extern "C" fn emulator_demo_step_v1(x: u32, reset: u32) -> DemoStatus {
    guarded(|state| {
        state
            .as_mut()
            .ok_or_else(|| "No demo on this thread".into())
            .and_then(|d| d.step(x, reset))
    })
}
#[no_mangle]
pub extern "C" fn emulator_demo_stop_v1() -> DemoStatus {
    guarded(|state| match state.take() {
        Some(d) => d.stop(),
        None => Ok(DemoStatus::default()),
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    static CALLS: AtomicU32 = AtomicU32::new(0);
    extern "C" fn oracle(frame: LiveDraw) -> u32 {
        CALLS.fetch_add(1, Ordering::Relaxed);
        if frame.abi_version != 1 || frame.draw_count != 2 || frame.image_bytes != 16384 {
            return 1;
        }
        // Axis-aligned integer rectangles: independently derive coverage from input vertices.
        for y in 0..64 {
            for x in 0..64 {
                let mut expected = [0, 0, 0, 255];
                for (i, draw) in frame.draws[..2].iter().enumerate() {
                    let at = |j| ((f32::from_bits(draw.vertices[j]) + 1.) * 32.) as usize;
                    if x >= at(0) && x < at(4) && y >= at(1) && y < at(9) {
                        expected = if i == 0 {
                            [0, 255, 0, 255]
                        } else {
                            [255, 0, 0, 255]
                        };
                    }
                }
                if frame.pixels[(y * 64 + x) * 4..(y * 64 + x + 1) * 4] != expected {
                    return 2;
                }
            }
        }
        0
    }
    extern "C" fn failure(_: LiveDraw) -> u32 {
        9
    }
    #[test]
    fn original_homebrew_animates_catches_misses_resets_and_releases() {
        assert_eq!(std::mem::size_of::<LiveDraw>(), 66068);
        assert_eq!(std::mem::size_of::<DemoStatus>(), 296);
        assert_eq!(std::mem::offset_of!(LiveDraw, pixels), 49684);
        let mut d = DemoSession::new(oracle).unwrap();
        for i in 1..=54 {
            let s = d.step(20, 0).unwrap();
            assert_eq!(s.frame, i);
        }
        assert_eq!(d.status().score, 1);
        for _ in 0..54 {
            d.step(0, 0).unwrap();
        }
        assert_eq!(d.status().misses, 1);
        let s = d.step(48, 1).unwrap();
        assert_eq!((s.frame, s.score, s.misses, s.paddle_x), (1, 0, 0, 48));
        // More than the previous fixed trace ceiling; explicit drain keeps memory bounded.
        for _ in 0..4100 {
            d.step(24, 0).unwrap();
        }
        assert!(d.session.kernel().ipc_records().is_empty());
        d.stop().unwrap();
    }
    #[test]
    fn failed_host_completion_never_publishes_pixels_reply_or_fence() {
        let mut d = DemoSession::new(failure).unwrap();
        assert!(d.step(24, 0).unwrap_err().contains("Host rendering"));
        assert_eq!(d.word(0), 0);
        assert_eq!(d.word(16), 0);
        let mut pixels = [0; 16384];
        d.session
            .memory()
            .read(LOAD_BASE + 0x6000, &mut pixels)
            .unwrap();
        assert_eq!(pixels, [0; 16384]);
        // Faulted sessions still release all owned state when dropped.
        drop(d);
        let mut d = DemoSession::new(oracle).unwrap();
        d.step(24, 0).unwrap();
        d.stop().unwrap();
    }
    #[test]
    fn thread_affinity_duplicate_start_bad_inputs_and_callback_reentry_are_bounded() {
        extern "C" fn reentry(frame: LiveDraw) -> u32 {
            if emulator_demo_step_v1(0, 0).status == 0 {
                return 5;
            }
            oracle(frame)
        }
        assert_eq!(emulator_demo_step_v1(0, 0).status, 1);
        assert_eq!(emulator_demo_start_v1(reentry).status, 0);
        assert_eq!(emulator_demo_start_v1(oracle).status, 1);
        assert_eq!(emulator_demo_step_v1(49, 0).status, 1);
        assert_eq!(emulator_demo_step_v1(24, 2).status, 1);
        assert_eq!(emulator_demo_step_v1(24, 0).status, 0);
        assert_eq!(emulator_demo_stop_v1().status, 0);
        assert_eq!(emulator_demo_stop_v1().status, 0);
    }
}
