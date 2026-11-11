use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    Auto,
    Hold,
    Toggle,
}

impl Mode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "Automatic tap / hold",
            Self::Hold => "Hold to talk",
            Self::Toggle => "Tap to toggle",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    Start,
    Stop,
    None,
}

#[derive(Default)]
pub struct Activation {
    down: Option<Instant>,
    latched: bool,
    ignore_release: bool,
}

impl Activation {
    pub fn press(&mut self, now: Instant, mode: Mode, recording: bool) -> Action {
        if self.down.is_some() {
            return Action::None;
        }
        self.down = Some(now);
        if recording && (self.latched || mode == Mode::Toggle) {
            self.latched = false;
            self.ignore_release = true;
            Action::Stop
        } else if !recording {
            self.ignore_release = false;
            Action::Start
        } else {
            Action::None
        }
    }
    pub fn release(&mut self, now: Instant, mode: Mode, threshold: Duration) -> Action {
        let Some(down) = self.down.take() else {
            return Action::None;
        };
        if self.ignore_release {
            self.ignore_release = false;
            return Action::None;
        }
        match mode {
            Mode::Hold => Action::Stop,
            Mode::Auto if now.duration_since(down) >= threshold => Action::Stop,
            _ => {
                self.latched = true;
                Action::None
            }
        }
    }
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tap_toggles_and_hold_submits() {
        let t = Instant::now();
        let limit = Duration::from_millis(250);
        let mut a = Activation::default();
        assert_eq!(a.press(t, Mode::Auto, false), Action::Start);
        assert_eq!(a.press(t, Mode::Auto, true), Action::None);
        assert_eq!(
            a.release(t + Duration::from_millis(50), Mode::Auto, limit),
            Action::None
        );
        assert_eq!(a.press(t + limit, Mode::Auto, true), Action::Stop);
        assert_eq!(
            a.release(t + limit + limit, Mode::Auto, limit),
            Action::None
        );
        a.reset();
        assert_eq!(a.press(t, Mode::Auto, false), Action::Start);
        assert_eq!(a.release(t + limit, Mode::Auto, limit), Action::Stop);
    }
    #[test]
    fn explicit_modes_and_stray_release() {
        let t = Instant::now();
        let limit = Duration::from_millis(250);
        let mut a = Activation::default();
        assert_eq!(a.release(t, Mode::Auto, limit), Action::None);
        a.press(t, Mode::Hold, false);
        assert_eq!(a.release(t, Mode::Hold, limit), Action::Stop);
        a.press(t, Mode::Toggle, false);
        assert_eq!(a.release(t + limit, Mode::Toggle, limit), Action::None);
    }
}
