//! A frame profiler: `let _t = profile::scope("name");` times the rest of the block on the CPU, nested scopes show as a tree, and once a
//! second the average and the worst frame of every scope are published. F3 (or `prof` in the console) shows them over the picture;
//! `--profile` also writes them to the log. Off, a scope costs one atomic load. The graphics card's own time is not in these numbers:
//! a scope around `present` that is long means the CPU waits for it (or for the vertical sync).

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

static ON: AtomicBool = AtomicBool::new(false);
const WINDOW: Duration = Duration::from_secs(1);

pub fn enabled() -> bool {
    ON.load(Ordering::Relaxed)
}

pub fn set_enabled(on: bool) {
    ON.store(on, Ordering::Relaxed);
}

/// What a scope took, in milliseconds, over the last published second.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub name: &'static str,
    pub depth: u8,
    pub avg: f32,
    pub worst: f32,
}

struct Slot {
    name: &'static str,
    depth: u8,
    frame: Duration,
    sum: Duration,
    worst: Duration,
}

struct Profiler {
    slots: Vec<Slot>,
    depth: u8,
    frames: u32,
    since: Instant,
    shown: Vec<Row>,
    fps: f32,
    log: bool,
}

thread_local! {
    static PROFILER: RefCell<Profiler> = RefCell::new(Profiler { slots: Vec::new(), depth: 0, frames: 0, since: Instant::now(), shown: Vec::new(), fps: 0.0, log: false });
}

/// Times the block it lives in; made by `scope`.
pub struct Scope(Option<(usize, Instant)>);

pub fn scope(name: &'static str) -> Scope {
    if !enabled() {
        return Scope(None);
    }
    PROFILER.with(|p| {
        let mut p = p.borrow_mut();
        let depth = p.depth;
        p.depth += 1;
        // A scope has its place in the list from the first time it opens, so a parent comes before its children.
        let slot = match p.slots.iter().position(|s| s.name == name && s.depth == depth) {
            Some(i) => i,
            None => {
                p.slots.push(Slot { name, depth, frame: Duration::ZERO, sum: Duration::ZERO, worst: Duration::ZERO });
                p.slots.len() - 1
            }
        };
        Scope(Some((slot, Instant::now())))
    })
}

impl Scope {
    /// End this scope and begin another one at the same level (the phases of a function, one after the other).
    pub fn lap(&mut self, name: &'static str) {
        *self = Scope(None);
        *self = scope(name);
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        let Some((slot, began)) = self.0.take() else { return };
        let took = began.elapsed();
        PROFILER.with(|p| {
            let mut p = p.borrow_mut();
            p.depth = p.depth.saturating_sub(1);
            p.slots[slot].frame += took;
        });
    }
}

/// Adds `took` to the scope `name` at the current level, for a cost that is gathered over a loop (a scope for each pass would be too much).
pub fn add(name: &'static str, took: Duration) {
    if !enabled() {
        return;
    }
    drop(scope(name));
    PROFILER.with(|p| {
        let mut p = p.borrow_mut();
        let depth = p.depth;
        if let Some(s) = p.slots.iter_mut().find(|s| s.name == name && s.depth == depth) {
            s.frame += took;
        }
    });
}

/// Adds the time it lives to `total`, when the profiler is on.
pub struct Sum<'a>(Option<(&'a mut Duration, Instant)>);

pub fn sum(total: &mut Duration) -> Sum<'_> {
    Sum(enabled().then(|| (total, Instant::now())))
}

impl Drop for Sum<'_> {
    fn drop(&mut self) {
        if let Some((total, began)) = self.0.take() {
            *total += began.elapsed();
        }
    }
}

/// The frame is over: its scopes count, and once a second the averages are published (and logged with `--profile`).
pub fn end_frame() {
    if !enabled() {
        return;
    }
    PROFILER.with(|p| {
        let mut p = p.borrow_mut();
        p.frames += 1;
        for s in &mut p.slots {
            s.sum += s.frame;
            s.worst = s.worst.max(s.frame);
            s.frame = Duration::ZERO;
        }
        if p.since.elapsed() >= WINDOW {
            publish(&mut p);
        }
    });
}

fn publish(p: &mut Profiler) {
    let frames = p.frames.max(1) as f32;
    let ms = |d: Duration| d.as_secs_f32() * 1000.0;
    p.fps = frames / p.since.elapsed().as_secs_f32();
    p.shown = p.slots.iter().map(|s| Row { name: s.name, depth: s.depth, avg: ms(s.sum) / frames, worst: ms(s.worst) }).collect();
    for s in &mut p.slots {
        (s.sum, s.worst) = (Duration::ZERO, Duration::ZERO);
    }
    (p.frames, p.since) = (0, Instant::now());
    if p.log {
        log::info!("profile, {:.0} fps (ms per frame: average / worst)\n{}", p.fps, table(&p.shown));
    }
}

/// Also write every published second to the log.
pub fn log_to_file(on: bool) {
    PROFILER.with(|p| p.borrow_mut().log = on);
}

/// The last published second: the frames per second and the scopes in the order they first ran.
pub fn report() -> (f32, Vec<Row>) {
    PROFILER.with(|p| {
        let p = p.borrow();
        (p.fps, p.shown.clone())
    })
}

fn table(rows: &[Row]) -> String {
    rows.iter().map(|r| format!("{:<40} {:6.2} {:6.2}", format!("{}{}", "  ".repeat(r.depth as usize), r.name), r.avg, r.worst)).collect::<Vec<_>>().join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_scopes_are_summed_per_frame_and_published_parent_first() {
        set_enabled(true);
        for _ in 0..3 {
            {
                let _frame = scope("frame");
                {
                    let _a = scope("a");
                    std::thread::sleep(Duration::from_millis(2));
                }
                let mut b = scope("b");
                std::thread::sleep(Duration::from_millis(1));
                b.lap("c");
            }
            // Publish now instead of after a second.
            PROFILER.with(|p| p.borrow_mut().since = Instant::now() - WINDOW);
            end_frame();
        }
        let (_, rows) = report();
        set_enabled(false);
        assert_eq!(rows.iter().map(|r| (r.name, r.depth)).collect::<Vec<_>>(), [("frame", 0), ("a", 1), ("b", 1), ("c", 1)]);
        let a = &rows[1];
        assert!(a.avg >= 1.5 && a.avg < 100.0 && a.worst >= a.avg, "{a:?}");
        assert!(rows[0].avg >= a.avg, "the frame holds its scopes");
    }
}
