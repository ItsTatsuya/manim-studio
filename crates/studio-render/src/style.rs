//! House styles: the engine themes behind Settings → House styles.
//!
//! A house style is a manim-rust theme (Manim Studio's style JSON). The host applies it
//! through `Config::theme`, so scene scripts never hard-code colours: they read
//! `this.theme.colors` and use text roles (`role: "title"`, `"body"`, `"hand"`, ...).

use manim::theme::Theme;

/// One house style.
#[derive(Clone, Copy, Debug)]
pub struct HouseStyle {
    pub id: &'static str,
    pub name: &'static str,
    pub json: &'static str,
}

/// The bundled styles, in picker order. Preferences and videos store an index into this
/// list; `studio_app::model::HOUSE_STYLES` must stay in the same order (a test checks).
pub const STYLES: [HouseStyle; 6] = [
    HouseStyle {
        id: "slate",
        name: "Slate",
        json: r##"{
  "id": "slate",
  "name": "Slate",
  "fonts": {
    "heading": "Fraunces",
    "body": "Source Sans 3",
    "code": "JetBrains Mono",
    "hand": "Kalam",
    "math": "auto",
    "fallback": [
      "Segoe UI Symbol",
      "Nirmala UI"
    ]
  },
  "colors": {
    "background": "#1D2622",
    "text": "#EDEADF",
    "muted": "#9DAAA1",
    "primary": "#F3CF6B",
    "secondary": "#8CC8E6",
    "accent": "#EE9A8C",
    "positive": "#9AD7A8",
    "negative": "#EE7F74",
    "key_fill": "#27332D",
    "example_fill": "#22303A"
  },
  "sizes": {
    "title": 56,
    "heading": 40,
    "body": 32,
    "small": 24
  },
  "layout": {
    "margin": 0.65,
    "title": "top-left",
    "boxes": "card",
    "accent": true,
    "pattern": "none",
    "pattern_spacing": 0.5
  },
  "pacing": {
    "run_time": 1,
    "wait": 1,
    "words_per_second": 2.4
  }
}"##,
    },
    HouseStyle {
        id: "whiteboard",
        name: "Whiteboard",
        json: r##"{
  "id": "whiteboard",
  "name": "Whiteboard",
  "fonts": {
    "heading": "Kalam",
    "body": "Andika",
    "code": "Space Mono",
    "hand": "Kalam",
    "math": "auto",
    "fallback": [
      "Segoe UI Symbol",
      "Nirmala UI"
    ]
  },
  "colors": {
    "background": "#FBFCFD",
    "text": "#1B1F24",
    "muted": "#5F6670",
    "primary": "#1F5FCF",
    "secondary": "#10865A",
    "accent": "#D12F2F",
    "positive": "#10865A",
    "negative": "#D12F2F",
    "key_fill": "#E8F0FC",
    "example_fill": "#E5F4EC"
  },
  "sizes": {
    "title": 58,
    "heading": 42,
    "body": 33,
    "small": 25
  },
  "layout": {
    "margin": 0.65,
    "title": "top-left",
    "boxes": "outline",
    "accent": false,
    "pattern": "none",
    "pattern_spacing": 0.5
  },
  "pacing": {
    "run_time": 1,
    "wait": 1.1,
    "words_per_second": 2.3
  }
}"##,
    },
    HouseStyle {
        id: "clean",
        name: "Clean",
        json: r##"{
  "id": "clean",
  "name": "Clean",
  "fonts": {
    "heading": "Plus Jakarta Sans",
    "body": "Inter",
    "code": "JetBrains Mono",
    "hand": "Caveat",
    "math": "auto",
    "fallback": [
      "Segoe UI Symbol",
      "Nirmala UI"
    ]
  },
  "colors": {
    "background": "#FFFFFF",
    "text": "#111827",
    "muted": "#6B7280",
    "primary": "#0F766E",
    "secondary": "#2563EB",
    "accent": "#B45309",
    "positive": "#16A34A",
    "negative": "#DC2626",
    "key_fill": "#E7F5F3",
    "example_fill": "#EAF0FE"
  },
  "sizes": {
    "title": 54,
    "heading": 40,
    "body": 32,
    "small": 24
  },
  "layout": {
    "margin": 0.65,
    "title": "top-left",
    "boxes": "card",
    "accent": true,
    "pattern": "grid",
    "pattern_spacing": 0.5
  },
  "pacing": {
    "run_time": 1,
    "wait": 1,
    "words_per_second": 2.4
  }
}"##,
    },
    HouseStyle {
        id: "phosphor",
        name: "Phosphor",
        json: r##"{
  "id": "phosphor",
  "name": "Phosphor",
  "fonts": {
    "heading": "JetBrains Mono",
    "body": "IBM Plex Sans",
    "code": "JetBrains Mono",
    "hand": "Caveat",
    "math": "auto",
    "fallback": [
      "Segoe UI Symbol",
      "Nirmala UI"
    ]
  },
  "colors": {
    "background": "#15120C",
    "text": "#F2E6C9",
    "muted": "#A89C80",
    "primary": "#FFB547",
    "secondary": "#8FD694",
    "accent": "#F28C5B",
    "positive": "#A6E3A1",
    "negative": "#F2766B",
    "key_fill": "#221D13",
    "example_fill": "#1A2015"
  },
  "sizes": {
    "title": 44,
    "heading": 34,
    "body": 30,
    "small": 23
  },
  "layout": {
    "margin": 0.6,
    "title": "top-left",
    "boxes": "outline",
    "accent": true,
    "pattern": "grid",
    "pattern_spacing": 0.5
  },
  "pacing": {
    "run_time": 0.9,
    "wait": 1,
    "words_per_second": 2.5
  }
}"##,
    },
    HouseStyle {
        id: "playground",
        name: "Playground",
        json: r##"{
  "id": "playground",
  "name": "Playground",
  "fonts": {
    "heading": "Fredoka",
    "body": "Nunito",
    "code": "Space Mono",
    "hand": "Patrick Hand",
    "math": "auto",
    "fallback": [
      "Segoe UI Symbol",
      "Nirmala UI"
    ]
  },
  "colors": {
    "background": "#FFF7E8",
    "text": "#23203A",
    "muted": "#6E6880",
    "primary": "#C93F14",
    "secondary": "#14998C",
    "accent": "#6545E0",
    "positive": "#2FA84F",
    "negative": "#E5383B",
    "key_fill": "#FFE3D4",
    "example_fill": "#D9F3EE"
  },
  "sizes": {
    "title": 60,
    "heading": 44,
    "body": 36,
    "small": 27
  },
  "layout": {
    "margin": 0.7,
    "title": "top-center",
    "boxes": "card",
    "accent": false,
    "pattern": "dots",
    "pattern_spacing": 0.7
  },
  "pacing": {
    "run_time": 1.2,
    "wait": 1.4,
    "words_per_second": 2
  }
}"##,
    },
    HouseStyle {
        id: "documentary",
        name: "Documentary",
        json: r##"{
  "id": "documentary",
  "name": "Documentary",
  "fonts": {
    "heading": "DM Serif Display",
    "body": "DM Sans",
    "code": "DM Mono",
    "hand": "Caveat",
    "math": "auto",
    "fallback": [
      "Segoe UI Symbol",
      "Nirmala UI"
    ]
  },
  "colors": {
    "background": "#121110",
    "text": "#F1EDE4",
    "muted": "#A39D92",
    "primary": "#E9B44C",
    "secondary": "#8DB8C8",
    "accent": "#E07A5F",
    "positive": "#9CC59A",
    "negative": "#E26D5A",
    "key_fill": "#1E1C19",
    "example_fill": "#1A2024"
  },
  "sizes": {
    "title": 58,
    "heading": 42,
    "body": 31,
    "small": 23
  },
  "layout": {
    "margin": 0.8,
    "title": "top-center",
    "boxes": "outline",
    "accent": false,
    "pattern": "none",
    "pattern_spacing": 0.5
  },
  "pacing": {
    "run_time": 1.2,
    "wait": 1.3,
    "words_per_second": 2.2
  }
}"##,
    },
];

/// The style at `index` (clamped), as JSON.
pub fn json(index: usize) -> &'static str {
    STYLES[index.min(STYLES.len() - 1)].json
}

/// The style at `index` (clamped), parsed.
pub fn theme(index: usize) -> Theme {
    Theme::from_json(json(index)).expect("bundled house styles are valid")
}

#[cfg(test)]
mod tests {
    #[test]
    fn styles_parse() {
        for (i, s) in super::STYLES.iter().enumerate() {
            let t = super::theme(i);
            assert_eq!(t.name, s.name);
        }
    }
}
