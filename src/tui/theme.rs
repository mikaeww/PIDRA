use ratatui::style::{Modifier, Style};

#[derive(Debug, Clone, Copy)]
pub struct Palette;

impl Palette {
    #[must_use]
    pub fn new(_no_color: bool) -> Self {
        Self
    }

    pub fn base(self) -> Style {
        Style::default()
    }

    pub fn warning(self) -> Style {
        Style::default().add_modifier(Modifier::BOLD)
    }

    pub fn danger(self) -> Style {
        Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
    }

    #[must_use]
    pub fn header(self) -> Style {
        self.accent().add_modifier(Modifier::BOLD)
    }

    #[must_use]
    pub fn table_header(self) -> Style {
        Style::default().add_modifier(Modifier::UNDERLINED)
    }

    #[must_use]
    pub fn selected_row(self) -> Style {
        self.base()
            .add_modifier(Modifier::REVERSED | Modifier::BOLD)
    }

    #[must_use]
    pub fn focused_action(self) -> Style {
        self.accent()
            .add_modifier(Modifier::REVERSED | Modifier::BOLD)
    }

    #[must_use]
    pub fn status(self) -> Style {
        self.muted()
    }

    #[must_use]
    pub fn footer(self) -> Style {
        self.muted()
    }

    pub fn accent(self) -> Style {
        Style::default().add_modifier(Modifier::BOLD)
    }

    pub fn muted(self) -> Style {
        Style::default().add_modifier(Modifier::DIM)
    }
}
