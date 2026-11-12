//! The text each dashboard row and the `--status` file show.
use crate::game::fishing::FishingSample;
use crate::r#macro::engine::EngineView;

pub fn rod_text(s: Option<&FishingSample>) -> String {
    match s {
        None => "unknown",
        Some(s) if s.open => "equipped (fishing)",
        Some(FishingSample { rod: None, .. }) => "unknown (no character)",
        Some(FishingSample { rod: Some(true), .. }) => "equipped",
        Some(_) => "not equipped",
    }
    .into()
}

pub fn minigame_text(s: Option<&FishingSample>) -> String {
    match s {
        None => "not found (not in game?)".into(),
        Some(s) if !s.open => "closed".into(),
        Some(s) => format!("open - progress {:.0}%", s.progress * 100.0),
    }
}

pub fn session_text(v: &EngineView) -> String {
    let o = format!("{} caught, {} lost, {} clicks", v.catches, v.losses, v.clicks);
    if v.last.is_empty() {
        o
    } else {
        format!("{o} (last: {})", v.last)
    }
}

/// A float as the `--status` file prints it (%g with 6 significant digits): 503.5, 1234.57.
/// shortcut: values outside 1e-4..1e6 print in plain notation, not %g's exponent form; screen pixels never get there.
pub fn float_text(v: f32) -> String {
    if v == 0.0 || !v.is_finite() {
        return if v == 0.0 { "0".into() } else { v.to_string() };
    }
    let decimals = (5 - v.abs().log10().floor() as i32).max(0) as usize;
    let s = format!("{v:.decimals$}");
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').into()
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn float_text_matches_printf_g() {
        assert_eq!(float_text(503.5), "503.5");
        assert_eq!(float_text(1234.5678), "1234.57");
        assert_eq!(float_text(24.0), "24");
        assert_eq!(float_text(-0.25), "-0.25");
        assert_eq!(float_text(0.0), "0");
    }
    use handy_sdk::input::key::vk_name;
    use windows::Win32::UI::Input::KeyboardAndMouse::VK_F1;

    #[test]
    fn parse_bounded() {
        use handy_sdk::util::text::parse_bounded;
        assert_eq!(parse_bounded("15", 0, 100), Some(15));
        assert!(parse_bounded("101", 0, 100).is_none());
        assert!(parse_bounded("4", 5, 300).is_none());
        assert!(parse_bounded("", 0, 9).is_none());
        assert!(parse_bounded("1a", 0, 99).is_none());
    }

    #[test]
    fn view_texts() {
        assert_eq!(rod_text(None), "unknown");
        let mut s = FishingSample {
            rod: Some(true),
            ..Default::default()
        };
        assert_eq!(rod_text(Some(&s)), "equipped");
        assert_eq!(minigame_text(Some(&s)), "closed");
        s.open = true;
        s.progress = 0.5;
        assert_eq!(minigame_text(Some(&s)), "open - progress 50%");
        assert_eq!(vk_name(u32::from(VK_F1.0)), "F1");
    }
}
