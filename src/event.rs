use std::{io, time::Duration};

use crossterm::event::{self, Event, MouseButton, MouseEventKind};
use ratatui::{Terminal, backend::Backend, layout::Rect};

use crate::{
    app::{App, AppView},
    control::{ControlWorker, RestartWorker},
    process::ScanWorker,
    tui,
};

pub fn run<B: Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
    scanner: &ScanWorker,
    control: &ControlWorker,
    restart: &RestartWorker,
    options: tui::RenderOptions,
    refresh_interval: Duration,
) -> io::Result<()>
where
    B::Error: std::error::Error + Send + Sync + 'static,
{
    let mut dirty = true;
    let mut frame_area = Rect::default();
    let mut last_draw = std::time::Instant::now()
        .checked_sub(refresh_interval)
        .unwrap_or_else(std::time::Instant::now);

    while !app.should_quit {
        let requests: Vec<_> = app.take_control_requests().collect();
        for request in requests {
            if let Err(error) = control.request(request) {
                app.report_control_dispatch_error(&error);
            }
        }
        while let Some(result) = control.try_result() {
            app.apply_control_result(result);
            dirty = true;
        }
        let restart_requests: Vec<_> = app.take_restart_requests().collect();
        for request in restart_requests {
            if let Err(error) = restart.request(request) {
                app.report_restart_dispatch_error(&error);
            }
        }
        while let Some(result) = restart.try_result() {
            app.apply_restart_result(result);
            dirty = true;
        }

        if let Some(message) = scanner.try_latest() {
            let _captured_at = message.captured_at;
            match message.result {
                Ok(batch) => app.apply_scan_batch(batch),
                Err(error) => app.report_scan_error(&error),
            }
            dirty = true;
        }

        if dirty || last_draw.elapsed() >= refresh_interval {
            terminal
                .draw(|frame| {
                    frame_area = frame.area();
                    tui::render(frame, app, options);
                })
                .map_err(io::Error::other)?;
            dirty = false;
            last_draw = std::time::Instant::now();
        }

        let poll_timeout = next_poll_timeout(refresh_interval, last_draw.elapsed());
        if event::poll(poll_timeout)? {
            match event::read()? {
                Event::Key(key) => {
                    app.handle_key(key);
                    dirty = true;
                }
                Event::Resize(_, _) => dirty = true,
                Event::Mouse(mouse) => {
                    match mouse.kind {
                        MouseEventKind::ScrollUp
                            if matches!(app.view, AppView::Details | AppView::RestartConfirm) =>
                        {
                            app.info_scroll.set(app.info_scroll.get().saturating_sub(3))
                        }
                        MouseEventKind::ScrollDown
                            if matches!(app.view, AppView::Details | AppView::RestartConfirm) =>
                        {
                            app.info_scroll.set(app.info_scroll.get().saturating_add(3))
                        }
                        MouseEventKind::ScrollUp => app.select_previous(),
                        MouseEventKind::ScrollDown => app.select_next(),
                        MouseEventKind::Down(MouseButton::Left) => {
                            if let Some(hit) =
                                tui::table_hit(frame_area, app, mouse.column, mouse.row)
                            {
                                app.select_from_pointer(hit.row, hit.focus);
                            }
                        }
                        MouseEventKind::Down(MouseButton::Right | MouseButton::Middle)
                        | MouseEventKind::Up(_)
                        | MouseEventKind::Drag(_)
                        | MouseEventKind::Moved
                        | MouseEventKind::ScrollLeft
                        | MouseEventKind::ScrollRight => {}
                    }
                    dirty = true;
                }
                Event::FocusGained | Event::FocusLost | Event::Paste(_) => {}
            }
        }
    }

    Ok(())
}

fn next_poll_timeout(refresh_interval: Duration, since_last_draw: Duration) -> Duration {
    refresh_interval
        .saturating_sub(since_last_draw)
        .min(Duration::from_millis(100))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::next_poll_timeout;

    #[test]
    fn idle_event_poll_blocks_instead_of_busy_looping() {
        assert_eq!(
            next_poll_timeout(Duration::from_secs(1), Duration::from_millis(10)),
            Duration::from_millis(100)
        );
        assert_eq!(
            next_poll_timeout(Duration::from_secs(1), Duration::from_secs(1)),
            Duration::ZERO
        );
    }
}
