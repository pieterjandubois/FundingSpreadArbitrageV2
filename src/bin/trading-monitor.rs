use std::error::Error;
use std::time::Duration;
use std::collections::HashMap;
use ratatui::{
    backend::CrosstermBackend,
    crossterm::{
        event::{self, Event, KeyCode},
        execute,
        terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    },
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::Span,
    widgets::{Block, Borders, Table, Row, Paragraph},
    Terminal,
};
use std::io;

type DynError = Box<dyn Error + Send + Sync>;

#[derive(Debug, Clone, serde::Deserialize)]
struct PortfolioMetrics {
    total_trades: u32,
    win_rate: f64,
    cumulative_pnl: f64,
    pnl_percentage: f64,
    available_capital: f64,
    utilization_pct: f64,
    leg_out_count: u32,
    leg_out_loss_pct: f64,
    realistic_apr: f64,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct PortfolioState {
    starting_capital: f64,
    available_capital: f64,
    total_open_positions: f64,
    active_trades: Vec<serde_json::Value>,
    closed_trades: Vec<serde_json::Value>,
    #[allow(dead_code)]
    cumulative_pnl: f64,
    #[allow(dead_code)]
    win_count: u32,
    #[allow(dead_code)]
    loss_count: u32,
    #[allow(dead_code)]
    leg_out_count: u32,
    #[allow(dead_code)]
    leg_out_total_loss: f64,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct TradeMetrics {
    trade_id: String,
    symbol: String,
    current_spread_bps: f64,
    unrealized_pnl: f64,
    current_long_price: f64,
    current_short_price: f64,
}

struct AppState {
    metrics: Option<PortfolioMetrics>,
    state: Option<PortfolioState>,
    trade_metrics: HashMap<String, TradeMetrics>,
    should_quit: bool,
    active_scroll_offset: usize,
    exits_scroll_offset: usize,
}

impl AppState {
    fn new() -> Self {
        Self {
            metrics: None,
            state: None,
            trade_metrics: HashMap::new(),
            should_quit: false,
            active_scroll_offset: 0,
            exits_scroll_offset: 0,
        }
    }

    async fn update_from_redis(&mut self) -> Result<(), DynError> {
        let client = redis::Client::open("redis://127.0.0.1:6379")?;
        let mut conn = client.get_connection()?;

        // Fetch metrics
        if let Ok(json) = redis::cmd("GET")
            .arg("strategy:portfolio:metrics")
            .query::<String>(&mut conn)
        {
            self.metrics = serde_json::from_str(&json).ok();
        }

        // Fetch portfolio state
        if let Ok(json) = redis::cmd("GET")
            .arg("strategy:portfolio:state")
            .query::<String>(&mut conn)
        {
            self.state = serde_json::from_str(&json).ok();
        }

        // Calculate trade metrics from active trades and current prices
        if let Some(state) = &self.state {
            for trade in &state.active_trades {
                if let Ok(trade_obj) = serde_json::from_value::<serde_json::Value>(trade.clone()) {
                    if let (Some(trade_id), Some(symbol), Some(entry_long), Some(entry_short), Some(entry_spread), Some(long_ex), Some(short_ex), Some(position_size)) = (
                        trade_obj.get("id").and_then(|v| v.as_str()),
                        trade_obj.get("symbol").and_then(|v| v.as_str()),
                        trade_obj.get("entry_long_price").and_then(|v| v.as_f64()),
                        trade_obj.get("entry_short_price").and_then(|v| v.as_f64()),
                        trade_obj.get("entry_spread_bps").and_then(|v| v.as_f64()),
                        trade_obj.get("long_exchange").and_then(|v| v.as_str()),
                        trade_obj.get("short_exchange").and_then(|v| v.as_str()),
                        trade_obj.get("position_size_usd").and_then(|v| v.as_f64()),
                    ) {
                        // Fetch current prices from the SPECIFIC exchanges used in the trade
                        let mut current_long = entry_long;
                        let mut current_short = entry_short;
                        
                        // Get exchange-specific parsers
                        let long_parser = arbitrage2::exchange_parser::get_parser(long_ex);
                        let short_parser = arbitrage2::exchange_parser::get_parser(short_ex);
                        
                        // Get long price (ASK price - what we pay to buy)
                        let long_patterns = arbitrage2::exchange_parser::get_redis_key_patterns(long_ex, symbol);
                        for key in long_patterns {
                            if let Ok(data) = redis::cmd("GET").arg(&key).query::<String>(&mut conn) {
                                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&data) {
                                    if let Some(ask_str) = long_parser.parse_ask(&json) {
                                        if let Ok(ask) = ask_str.parse::<f64>() {
                                            current_long = ask;
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                        
                        // Get short price (BID price - what we get to sell)
                        let short_patterns = arbitrage2::exchange_parser::get_redis_key_patterns(short_ex, symbol);
                        for key in short_patterns {
                            if let Ok(data) = redis::cmd("GET").arg(&key).query::<String>(&mut conn) {
                                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&data) {
                                    if let Some(bid_str) = short_parser.parse_bid(&json) {
                                        if let Ok(bid) = bid_str.parse::<f64>() {
                                            current_short = bid;
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                        
                        // Calculate current spread: (short_bid - long_ask) / long_ask * 10000
                        let current_spread_bps = if current_long > 0.0 {
                            ((current_short - current_long) / current_long) * 10000.0
                        } else {
                            entry_spread
                        };
                        
                        // Calculate unrealized P&L based on spread reduction
                        let spread_reduction_bps = entry_spread - current_spread_bps;
                        
                        // P&L = spread_reduction / 10000 * position_size
                        // Fees are already accounted for in the projected profit target
                        let unrealized_pnl = (spread_reduction_bps / 10000.0) * position_size;
                        
                        let metrics = TradeMetrics {
                            trade_id: trade_id.to_string(),
                            symbol: symbol.to_string(),
                            current_spread_bps,
                            unrealized_pnl,
                            current_long_price: current_long,
                            current_short_price: current_short,
                        };
                        
                        self.trade_metrics.insert(trade_id.to_string(), metrics);
                    }
                }
            }
        }

        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<(), DynError> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app_state = AppState::new();
    let mut last_update = std::time::Instant::now();
    let update_interval = Duration::from_millis(200); // Update every 200ms for live feel

    loop {
        // Handle events with timeout
        if event::poll(Duration::from_millis(16))? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => {
                        app_state.should_quit = true;
                    }
                    KeyCode::Up => {
                        if app_state.active_scroll_offset > 0 {
                            app_state.active_scroll_offset -= 1;
                        }
                    }
                    KeyCode::Down => {
                        if let Some(state) = &app_state.state {
                            if app_state.active_scroll_offset < state.active_trades.len().saturating_sub(1) {
                                app_state.active_scroll_offset += 1;
                            }
                        }
                    }
                    KeyCode::PageUp => {
                        app_state.active_scroll_offset = app_state.active_scroll_offset.saturating_sub(5);
                    }
                    KeyCode::PageDown => {
                        if let Some(state) = &app_state.state {
                            app_state.active_scroll_offset = (app_state.active_scroll_offset + 5)
                                .min(state.active_trades.len().saturating_sub(1));
                        }
                    }
                    KeyCode::Home => {
                        app_state.active_scroll_offset = 0;
                    }
                    KeyCode::End => {
                        if let Some(state) = &app_state.state {
                            app_state.active_scroll_offset = state.active_trades.len().saturating_sub(1);
                        }
                    }
                    KeyCode::Char('j') => {
                        if app_state.exits_scroll_offset > 0 {
                            app_state.exits_scroll_offset -= 1;
                        }
                    }
                    KeyCode::Char('k') => {
                        if let Some(state) = &app_state.state {
                            if app_state.exits_scroll_offset < state.closed_trades.len().saturating_sub(1) {
                                app_state.exits_scroll_offset += 1;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        // Update data from Redis periodically
        if last_update.elapsed() >= update_interval {
            let _ = app_state.update_from_redis().await;
            last_update = std::time::Instant::now();
        }

        // Draw UI
        terminal.draw(|f| ui(f, &app_state))?;

        if app_state.should_quit {
            break;
        }
    }

    // Restore terminal
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    Ok(())
}

fn ui(f: &mut ratatui::Frame, app: &AppState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(5),
            Constraint::Min(8),
            Constraint::Min(8),
            Constraint::Length(2),
        ])
        .split(f.size());

    // Portfolio Summary
    render_portfolio_summary(f, chunks[0], app);

    // Active Trades
    render_active_trades(f, chunks[1], app);

    // Recent Exits
    render_recent_exits(f, chunks[2], app);

    // Footer
    render_footer(f, chunks[3]);
}

fn render_portfolio_summary(f: &mut ratatui::Frame, area: ratatui::layout::Rect, app: &AppState) {
    let mut summary_lines = vec![];

    if let Some(state) = &app.state {
        let utilization = (state.total_open_positions / state.starting_capital) * 100.0;
        summary_lines.push(format!(
            "Starting Capital: ${:.2} | Available: ${:.2} | In Positions: ${:.2} | Utilization: {:.1}%",
            state.starting_capital, state.available_capital, state.total_open_positions, utilization
        ));
    }

    if let Some(metrics) = &app.metrics {
        let line1 = format!(
            "Trades: {} | Win Rate: {:.1}% | Cumulative P&L: ${:.2} ({:+.2}%)",
            metrics.total_trades, metrics.win_rate, metrics.cumulative_pnl, metrics.pnl_percentage
        );
        summary_lines.push(line1);

        let line2 = format!(
            "Realistic APR: {:.1}% | Leg-Out Events: {} | Leg-Out Loss: {:.1}%",
            metrics.realistic_apr, metrics.leg_out_count, metrics.leg_out_loss_pct
        );
        summary_lines.push(line2);
    } else {
        summary_lines.push("Waiting for data...".to_string());
    }

    let text = summary_lines.join("\n");
    let paragraph = Paragraph::new(text)
        .style(Style::default().fg(Color::Cyan))
        .block(Block::default().borders(Borders::ALL).title("Portfolio Summary"));

    f.render_widget(paragraph, area);
}

fn render_active_trades(f: &mut ratatui::Frame, area: ratatui::layout::Rect, app: &AppState) {
    if let Some(state) = &app.state {
        let table_height = area.height as usize;
        let visible_rows: Vec<_> = state
            .active_trades
            .iter()
            .skip(app.active_scroll_offset)
            .take(table_height.saturating_sub(2))
            .collect();

        let rows: Vec<Row> = visible_rows
            .iter()
            .filter_map(|trade| {
                let trade_obj = serde_json::from_value::<serde_json::Value>((*trade).clone()).ok()?;
                let symbol = trade_obj.get("symbol").and_then(|v| v.as_str()).unwrap_or("N/A").to_string();
                let entry_spread = trade_obj.get("entry_spread_bps").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let position_size = trade_obj.get("position_size_usd").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let long_ex = trade_obj.get("long_exchange").and_then(|v| v.as_str()).unwrap_or("N/A").to_string();
                let short_ex = trade_obj.get("short_exchange").and_then(|v| v.as_str()).unwrap_or("N/A").to_string();
                let trade_id = trade_obj.get("id").and_then(|v| v.as_str()).unwrap_or("N/A").to_string();
                let projected_profit = trade_obj.get("projected_profit_usd").and_then(|v| v.as_f64()).unwrap_or(0.0);

                // Get live metrics from cache
                let (current_spread, unrealized_pnl) = if let Some(metrics) = app.trade_metrics.get(&trade_id) {
                    (metrics.current_spread_bps, metrics.unrealized_pnl)
                } else {
                    // Fallback: use entry spread and 0 P&L if metrics not yet available
                    (entry_spread, 0.0)
                };

                let pnl_color = if unrealized_pnl >= 0.0 { Color::Green } else { Color::Red };
                let pnl_display = if unrealized_pnl < 0.0 {
                    format!("-${:.2}", unrealized_pnl.abs())
                } else {
                    format!("${:.2}", unrealized_pnl)
                };

                Some(Row::new(vec![
                    Span::raw(symbol),
                    Span::raw(format!("{:.2}bps", entry_spread)),
                    Span::raw(format!("{:.2}bps", current_spread)),
                    Span::styled(pnl_display, Style::default().fg(pnl_color)),
                    Span::raw(format!("${:.2}", projected_profit)),
                    Span::raw(format!("${:.2}", position_size)),
                    Span::raw(long_ex),
                    Span::raw(short_ex),
                ]))
            })
            .collect();

        let table = Table::new(
            rows,
            [
                Constraint::Length(12),
                Constraint::Length(14),
                Constraint::Length(14),
                Constraint::Length(12),
                Constraint::Length(12),
                Constraint::Length(12),
                Constraint::Length(12),
                Constraint::Length(12),
            ],
        )
        .header(
            Row::new(vec![
                Span::styled("Ticker", Style::default().add_modifier(Modifier::BOLD)),
                Span::styled("Entry Spread", Style::default().add_modifier(Modifier::BOLD)),
                Span::styled("Current Spread", Style::default().add_modifier(Modifier::BOLD)),
                Span::styled("Unrealized P&L", Style::default().add_modifier(Modifier::BOLD)),
                Span::styled("Expected Profit", Style::default().add_modifier(Modifier::BOLD)),
                Span::styled("Position Size", Style::default().add_modifier(Modifier::BOLD)),
                Span::styled("Long Ex", Style::default().add_modifier(Modifier::BOLD)),
                Span::styled("Short Ex", Style::default().add_modifier(Modifier::BOLD)),
            ])
            .style(Style::default().fg(Color::Cyan)),
        )
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!("Active Trades ({} total)", state.active_trades.len())),
        );

        f.render_widget(table, area);
    } else {
        let paragraph = Paragraph::new("Waiting for data...")
            .block(Block::default().borders(Borders::ALL).title("Active Trades"));
        f.render_widget(paragraph, area);
    }
}

fn render_recent_exits(f: &mut ratatui::Frame, area: ratatui::layout::Rect, app: &AppState) {
    if let Some(state) = &app.state {
        let table_height = area.height as usize;
        let visible_rows: Vec<_> = state
            .closed_trades
            .iter()
            .rev()
            .skip(app.exits_scroll_offset)
            .take(table_height.saturating_sub(2))
            .collect();

        let rows: Vec<Row> = visible_rows
            .iter()
            .filter_map(|trade| {
                let trade_obj = serde_json::from_value::<serde_json::Value>((*trade).clone()).ok()?;
                let symbol = trade_obj.get("symbol").and_then(|v| v.as_str()).unwrap_or("N/A").to_string();
                let actual_profit = trade_obj.get("actual_profit_usd").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let exit_reason = trade_obj.get("exit_reason").and_then(|v| v.as_str()).unwrap_or("N/A").to_string();

                let pnl_color = if actual_profit >= 0.0 { Color::Green } else { Color::Red };
                let pnl_display = if actual_profit < 0.0 {
                    format!("-${:.2}", actual_profit.abs())
                } else {
                    format!("${:.2}", actual_profit)
                };

                Some(Row::new(vec![
                    Span::raw(symbol),
                    Span::styled(pnl_display, Style::default().fg(pnl_color)),
                    Span::raw(exit_reason),
                ]))
            })
            .collect();

        let table = Table::new(
            rows,
            [
                Constraint::Length(12),
                Constraint::Length(14),
                Constraint::Min(30),
            ],
        )
        .header(
            Row::new(vec![
                Span::styled("Ticker", Style::default().add_modifier(Modifier::BOLD)),
                Span::styled("Profit", Style::default().add_modifier(Modifier::BOLD)),
                Span::styled("Exit Reason", Style::default().add_modifier(Modifier::BOLD)),
            ])
            .style(Style::default().fg(Color::Cyan)),
        )
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!("Recent Exits ({} total)", state.closed_trades.len())),
        );

        f.render_widget(table, area);
    } else {
        let paragraph = Paragraph::new("Waiting for data...")
            .block(Block::default().borders(Borders::ALL).title("Recent Exits"));
        f.render_widget(paragraph, area);
    }
}

fn render_footer(f: &mut ratatui::Frame, area: ratatui::layout::Rect) {
    let footer_text = "↑↓: Scroll Active | j/k: Scroll Exits | PgUp/PgDn: Page | Home/End: Jump | q: Quit";
    let footer = Paragraph::new(footer_text)
        .style(Style::default().fg(Color::Gray))
        .block(Block::default().borders(Borders::TOP));
    f.render_widget(footer, area);
}
