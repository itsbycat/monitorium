use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard, PoisonError};

use objc2_core_graphics::{
    CGDirectDisplayID, CGDisplayGammaTableCapacity, CGError, CGGammaValue,
    CGGetDisplayTransferByTable, CGSetDisplayTransferByTable,
};

// the same floor as the overlay's maximum opacity of 235/255
const MIN_FACTOR: f32 = 1.0 - 235.0 / 255.0;
const TOLERANCE: f32 = 1.0 / 512.0;

// shared with the service thread, which re-applies tables macOS reset behind our back
static DIMMED: Mutex<BTreeMap<CGDirectDisplayID, Dimmed>> = Mutex::new(BTreeMap::new());

struct Dimmed {
    original: Tables,
    applied: Tables,
}

struct Tables {
    red: Vec<CGGammaValue>,
    green: Vec<CGGammaValue>,
    blue: Vec<CGGammaValue>,
}

impl Tables {
    fn scaled(&self, factor: f32) -> Self {
        Self {
            red: scale(&self.red, factor),
            green: scale(&self.green, factor),
            blue: scale(&self.blue, factor),
        }
    }

    fn matches(&self, other: &Self) -> bool {
        let close = |a: &[f32], b: &[f32]| {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| (a - b).abs() <= TOLERANCE)
        };
        close(&self.red, &other.red)
            && close(&self.green, &other.green)
            && close(&self.blue, &other.blue)
    }
}

fn dimmed() -> MutexGuard<'static, BTreeMap<CGDirectDisplayID, Dimmed>> {
    DIMMED.lock().unwrap_or_else(PoisonError::into_inner)
}

pub fn set_level(display: CGDirectDisplayID, level: u8) -> Result<(), String> {
    if level >= 100 {
        reset(display);
        return Ok(());
    }
    let mut dimmed = dimmed();
    // keep the table from before we first dimmed, never a dimmed one
    let original = match dimmed.remove(&display) {
        Some(previous) => previous.original,
        None => {
            read(display).ok_or_else(|| format!("reading the gamma table of {display} failed"))?
        }
    };
    let applied = original.scaled(factor(level));
    let result = write(display, &applied);
    dimmed.insert(display, Dimmed { original, applied });
    result
}

pub fn reset(display: CGDirectDisplayID) {
    let previous = dimmed().remove(&display);
    if let Some(previous) = previous
        && let Err(err) = write(display, &previous.original)
    {
        log::warn!("resetting gamma: {err}");
    }
}

pub fn reset_all() {
    let all = std::mem::take(&mut *dimmed());
    for (display, previous) in all {
        if let Err(err) = write(display, &previous.original) {
            log::warn!("resetting gamma: {err}");
        }
    }
}

/// Re-applies dimming that macOS dropped, which it does on wake, display changes and color
/// profile or Night Shift changes.
pub fn reassert() {
    let dimmed = dimmed();
    for (&display, entry) in dimmed.iter() {
        if read(display).is_some_and(|live| !live.matches(&entry.applied))
            && let Err(err) = write(display, &entry.applied)
        {
            log::warn!("re-applying gamma: {err}");
        }
    }
}

fn read(display: CGDirectDisplayID) -> Option<Tables> {
    let capacity = CGDisplayGammaTableCapacity(display);
    if capacity == 0 {
        return None;
    }
    let mut tables = Tables {
        red: vec![0.0; capacity as usize],
        green: vec![0.0; capacity as usize],
        blue: vec![0.0; capacity as usize],
    };
    let mut count = 0u32;
    let status = unsafe {
        CGGetDisplayTransferByTable(
            display,
            capacity,
            tables.red.as_mut_ptr(),
            tables.green.as_mut_ptr(),
            tables.blue.as_mut_ptr(),
            &mut count,
        )
    };
    if status != CGError::Success || count == 0 {
        return None;
    }
    for table in [&mut tables.red, &mut tables.green, &mut tables.blue] {
        table.truncate(count as usize);
    }
    Some(tables)
}

fn write(display: CGDirectDisplayID, tables: &Tables) -> Result<(), String> {
    let status = unsafe {
        CGSetDisplayTransferByTable(
            display,
            tables.red.len() as u32,
            tables.red.as_ptr(),
            tables.green.as_ptr(),
            tables.blue.as_ptr(),
        )
    };
    if status == CGError::Success {
        Ok(())
    } else {
        Err(format!(
            "CGSetDisplayTransferByTable({display}) failed ({})",
            status.0
        ))
    }
}

fn factor(level: u8) -> f32 {
    MIN_FACTOR + (1.0 - MIN_FACTOR) * f32::from(level.min(100)) / 100.0
}

fn scale(table: &[CGGammaValue], factor: f32) -> Vec<CGGammaValue> {
    table.iter().map(|value| value * factor).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn factor_matches_overlay_range() {
        assert_eq!(factor(100), 1.0);
        assert!((factor(0) - 20.0 / 255.0).abs() < 1e-6);
        assert!(factor(50) > factor(49));
    }

    #[test]
    fn scaling_keeps_the_curve() {
        let original = Tables {
            red: vec![0.0, 0.5, 1.0],
            green: vec![0.0, 0.4, 0.9],
            blue: vec![0.1, 0.5, 1.0],
        };
        let half = original.scaled(0.5);
        assert_eq!(half.red, vec![0.0, 0.25, 0.5]);
        assert_eq!(half.green, vec![0.0, 0.2, 0.45]);
        assert!(half.matches(&original.scaled(0.5)));
        assert!(!half.matches(&original));
    }
}
