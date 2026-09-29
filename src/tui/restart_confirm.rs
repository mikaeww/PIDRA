use ratatui::{
    Frame,
    layout::{Constraint, Layout},
    text::Line,
    widgets::Paragraph,
};

use crate::{
    app::App,
    control::restart::RestartSource,
    tui::{RenderOptions, theme::Palette},
};

pub fn render(frame: &mut Frame<'_>, app: &App, options: RenderOptions) {
    let palette = Palette::new(options.no_color);
    let rows = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(0),
        Constraint::Length(2),
    ])
    .split(frame.area());
    let Some(confirmation) = app.restart_confirmation.as_ref() else {
        frame.render_widget(
            Paragraph::new("Restart target unavailable\n\nEsc Cancel"),
            frame.area(),
        );
        return;
    };

    frame.render_widget(
        Paragraph::new(vec![
            Line::styled(
                format!("Restart {}", confirmation.process_name),
                palette.header(),
            ),
            Line::styled(
                format!("PID {}", confirmation.identity.pid),
                palette.muted(),
            ),
        ]),
        rows[0],
    );

    let mut lines = vec![
        Line::styled("Save your work first.", palette.warning()),
        Line::from(""),
        Line::styled("STEPS", palette.table_header()),
    ];
    match &confirmation.source {
        RestartSource::SystemdUserUnit { unit } => {
            lines.extend([
                Line::from("1  Ask systemd to restart the application."),
                Line::from("2  Wait for its new process."),
                Line::from(""),
                Line::styled(format!("Service: {unit}"), palette.muted()),
            ]);
        }
        RestartSource::Direct { .. } => {
            lines.extend([
                Line::from("1  Stop the selected process normally (SIGTERM)."),
                Line::from("2  Start the recorded executable again."),
                Line::from(""),
                Line::from("If the process stays open, PIDRA cancels the restart."),
                Line::from("PIDRA never force kills during restart."),
                Line::from(""),
                Line::styled(
                    "Unsaved work and the full environment cannot be restored.",
                    palette.warning(),
                ),
            ]);
        }
        RestartSource::Unavailable { reason } => {
            lines.push(Line::from(format!("Restart unavailable: {reason}")));
        }
    }
    lines.extend([
        Line::from(""),
        Line::styled("TARGET", palette.table_header()),
        Line::from(format!(
            "{}   PID {}",
            confirmation.process_name, confirmation.identity.pid
        )),
        Line::styled(
            "PID and start time are checked again before anything happens.",
            palette.muted(),
        ),
    ]);
    super::render_scrollable(frame, rows[1], lines, &app.info_scroll);
    let footer = if frame.area().width < 40 {
        "Enter Restart\nEsc Cancel  PgUp/PgDn"
    } else {
        "Enter / Y Restart   Esc / N Cancel\nPgUp/PgDn Scroll"
    };
    frame.render_widget(Paragraph::new(footer).style(palette.accent()), rows[2]);
}
