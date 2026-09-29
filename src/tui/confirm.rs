use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout},
    text::Line,
    widgets::Paragraph,
};

use crate::{
    app::App,
    control::risk::assess_termination,
    tui::{RenderOptions, theme::Palette},
};

pub fn render(frame: &mut Frame<'_>, app: &App, options: RenderOptions) {
    let palette = Palette::new(options.no_color);
    let area = frame.area();
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Fill(1),
            Constraint::Length(12),
            Constraint::Fill(1),
        ])
        .split(area);
    let Some(confirmation) = app.confirmation.as_ref() else {
        frame.render_widget(Paragraph::new("Confirmation target unavailable"), rows[1]);
        return;
    };
    let process = app.process_by_identity(confirmation.identity);
    let assessment = process.map(|process| {
        assess_termination(
            process,
            &app.all_processes,
            i32::try_from(std::process::id()).unwrap_or(i32::MAX),
        )
    });
    let lines = vec![
        Line::styled("CONFIRM FORCE STOP", palette.danger()),
        Line::from(""),
        Line::from(format!("PROCESS    {}", confirmation.process_name)),
        Line::from(format!("PID        {}", confirmation.identity.pid)),
        Line::from(format!(
            "START      {} ticks",
            confirmation.identity.start_time_ticks
        )),
        Line::from("TARGET     exact PID/start-time identity — not the whole process tree"),
        Line::from(format!(
            "ASSESSMENT {}",
            assessment
                .as_ref()
                .map_or("UNKNOWN", |value| value.rating.label())
        )),
        Line::from(""),
        Line::from(
            "SIGKILL gives the process no chance to save data or repair application databases.",
        ),
        Line::from("PIDRA will validate the start time again before sending the signal."),
        Line::from(""),
        Line::styled("ENTER / Y CONFIRM     ESC / N CANCEL", palette.danger()),
    ];
    frame.render_widget(Paragraph::new(lines).alignment(Alignment::Left), rows[1]);
}
