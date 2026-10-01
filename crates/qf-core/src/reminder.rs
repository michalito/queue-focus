//! The flash reminder: what one flash carries.

use crate::{FlashStyle, Intensity, Palette};

/// One flash, resolved: everything needed to draw it, so a shell that has
/// only just connected still draws the right thing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlashEvent {
    pub style: FlashStyle,
    pub intensity: Intensity,
    pub palette: Palette,
    pub title: String,
    /// Already formatted, so the flash and the top bar always read the same.
    pub timer: String,
}

impl FlashEvent {
    /// The `Flash` signal's payload, as the shell extension reads it.
    pub fn to_json(&self) -> String {
        serde_json::json!({
            "style": self.style.as_str(),
            "intensity": self.intensity.as_str(),
            "palette": self.palette.as_str(),
            "title": self.title,
            "timer": self.timer,
        })
        .to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_flash_carries_everything_needed_to_draw_it() {
        let event = FlashEvent {
            style: FlashStyle::EdgesSoft,
            intensity: Intensity::Strong,
            palette: Palette::Orange,
            title: "call \"mum\"".into(),
            timer: "1h02 ⏸".into(),
        };
        let json: serde_json::Value = serde_json::from_str(&event.to_json()).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "style": "edgesSoft",
                "intensity": "strong",
                "palette": "orange",
                "title": "call \"mum\"",
                "timer": "1h02 ⏸",
            })
        );
    }
}
