use crossterm::{
    cursor, execute, queue,
    style::{Color, Print, ResetColor, SetForegroundColor},
    terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
};
use std::{collections::VecDeque, io::{stdout, Write}};

pub struct DevConsole {
    pub active: bool,
    lines: VecDeque<String>,
    pub source: String,
    pub status: String,
    pub server_url: Option<String>,
    scroll: usize,
}

impl DevConsole {
    pub fn new(source: &str) -> Self {
        let active = terminal::enable_raw_mode().is_ok();
        if active {
            if execute!(stdout(), EnterAlternateScreen, cursor::Hide).is_err() {
                let _ = terminal::disable_raw_mode();
                return Self { active: false, lines: VecDeque::new(), source: source.into(), status: "STARTING".into(), server_url: None, scroll: 0 };
            }
        }
        Self { active, lines: VecDeque::new(), source: source.into(), status: "STARTING".into(), scroll: 0 }
    }

    pub fn log(&mut self, level: &str, message: &str) {
        if let Some(url) = message.strip_prefix("RUX native CRUD: ").and_then(|line| line.split_whitespace().next()) {
            if url.starts_with("http://") || url.starts_with("https://") {
                self.server_url = Some(url.to_string());
            }
        }
        let entry = format!("[{level}] {message}");
        if !self.active { println!("{entry}"); return; }
        for line in entry.lines() {
            if self.lines.len() >= 5000 { self.lines.pop_front(); }
            self.lines.push_back(line.to_string());
        }
        self.scroll = 0;
        self.render();
    }

    pub fn clear_endpoint(&mut self) { self.server_url = None; self.render(); }

    pub fn scroll_up(&mut self) { self.scroll = self.scroll.saturating_add(3); self.render(); }
    pub fn scroll_down(&mut self) { self.scroll = self.scroll.saturating_sub(3); self.render(); }

    pub fn render(&self) {
        if !self.active { return; }
        let Ok((width, height)) = terminal::size() else { return; };
        if height < 7 || width < 25 { return; }
        let mut out = stdout();
        let _ = queue!(out, cursor::MoveTo(0, 0), Clear(ClearType::All));
        let heading = format!(" RUX {}  |  DEVELOPMENT  |  {}", env!("CARGO_PKG_VERSION"), self.status);
        let _ = queue!(out, SetForegroundColor(Color::Cyan), Print(truncate(&heading, width)), ResetColor);
        let _ = queue!(out, cursor::MoveTo(0, 1), Print(truncate(&format!(" Source: {}", self.source), width)));
        let rule = "-".repeat(width as usize);
        let endpoint = self.server_url.as_deref().unwrap_or("Waiting for server...");
        let port = self.server_url.as_deref().and_then(|url| url.split("://").nth(1))
            .and_then(|authority| authority.split('/').next())
            .and_then(|authority| authority.rsplit_once(':'))
            .map(|(_, port)| port).unwrap_or("--");
        let _ = queue!(out, cursor::MoveTo(0, 2), SetForegroundColor(Color::Green),
            Print(truncate(&format!(" URL: {endpoint}  |  Port: {port}"), width)), ResetColor);
        let _ = queue!(out, cursor::MoveTo(0, 3), SetForegroundColor(Color::DarkGrey), Print(&rule), ResetColor);
        let available = (height - 6) as usize;
        let end = self.lines.len().saturating_sub(self.scroll.min(self.lines.len()));
        let start = end.saturating_sub(available);
        for (index, line) in self.lines.iter().skip(start).take(available).enumerate() {
            let color = if line.starts_with("[ERROR]") { Color::Red }
                else if line.starts_with("[WARN]") { Color::Yellow }
                else if line.starts_with("[DEBUG]") { Color::DarkGrey }
                else if line.starts_with("[SUCCESS]") { Color::Green }
                else if line.starts_with("[INFO]") { Color::Blue }
                else { Color::White };
            let _ = queue!(out, cursor::MoveTo(0, index as u16 + 4), SetForegroundColor(color), Print(truncate(line, width)), ResetColor);
        }
        let _ = queue!(out, cursor::MoveTo(0, height - 2), SetForegroundColor(Color::DarkGrey), Print(&rule), ResetColor);
        let actions = "[R] Restart  [T] Tests  [S] Status  [H] Help  [Q] Quit  [Up/Down] Scroll";
        let _ = queue!(out, cursor::MoveTo(0, height - 1), SetForegroundColor(Color::Cyan), Print(truncate(actions, width)), ResetColor);
        let _ = out.flush();
    }
}

fn truncate(value: &str, width: u16) -> String {
    value.chars().take(width as usize).collect()
}

impl Drop for DevConsole {
    fn drop(&mut self) {
        if self.active {
            let _ = execute!(stdout(), LeaveAlternateScreen, cursor::Show, ResetColor);
            let _ = terminal::disable_raw_mode();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn truncates_without_splitting_utf8() {
        assert_eq!(truncate("áéx", 2), "áé");
    }
}
