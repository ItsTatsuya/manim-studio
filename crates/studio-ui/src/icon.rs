use crate::theme::ActiveTheme;
use gpui::{App, Hsla, IntoElement, Pixels, RenderOnce, Styled, Window, px, svg};

macro_rules! icons {
    ($($variant:ident => $file:literal),* $(,)?) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum IconName { $($variant),* }

        impl IconName {
            pub fn path(self) -> &'static str {
                match self { $(IconName::$variant => concat!("icons/", $file, ".svg")),* }
            }
        }
    };
}

icons! {
    ArrowUpDown => "arrow-up-down",
    Board => "board",
    Captions => "cc",
    Check => "check",
    ChevronRight => "chev",
    ChevronDown => "chevd",
    ChevronLeft => "chevl",
    Claude => "claude",
    Clip => "clip",
    Crop => "crop",
    Doc => "doc",
    Dots => "dots",
    Download => "download",
    External => "external",
    Film => "film",
    Folder => "folder",
    FolderPlus => "folder-plus",
    Gauge => "gauge",
    Gear => "gear",
    Grid => "grid",
    Image => "image",
    Info => "info",
    Key => "key",
    Keyboard => "keyboard",
    Layers => "layers",
    Loop => "loop",
    Mic => "mic",
    Move => "move",
    Minus => "minus",
    Moon => "moon",
    Music => "music",
    OpenAi => "openai",
    Palette => "palette",
    Pencil => "pencil",
    Pause => "pause",
    Play => "play",
    Plus => "plus",
    Queue => "queue",
    Refresh => "refresh",
    Script => "script",
    Search => "search",
    Shield => "shield",
    Sidebar => "sidebar",
    Sliders => "sliders",
    Spark => "spark",
    Stop => "stop",
    Sun => "sun",
    Trash => "trash",
    Up => "up",
    Volume => "volume",
    VolumeLow => "volume-low",
    VolumeOff => "volume-x",
    WinClose => "win-close",
    WinMax => "win-max",
    WinMin => "win-min",
    X => "x",
}

/// A monochrome icon. GPUI renders SVGs as alpha masks, so color is explicit.
#[derive(IntoElement)]
pub struct Icon {
    name: IconName,
    size: Pixels,
    color: Option<Hsla>,
}

impl Icon {
    pub fn new(name: IconName) -> Self {
        Self {
            name,
            size: px(16.),
            color: None,
        }
    }

    pub fn size(mut self, size: Pixels) -> Self {
        self.size = size;
        self
    }

    pub fn small(self) -> Self {
        self.size(px(14.))
    }

    pub fn color(mut self, color: Hsla) -> Self {
        self.color = Some(color);
        self
    }
}

impl RenderOnce for Icon {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();
        let color = self.color.unwrap_or(match self.name {
            IconName::Claude => theme.claude,
            _ => theme.fg_2,
        });
        svg()
            .path(self.name.path())
            .size(self.size)
            .flex_none()
            .text_color(color)
    }
}
