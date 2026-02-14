use std::error::Error;
use std::collections::{BTreeMap, VecDeque};
use std::time::Duration;
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
    widgets::{Block, Borders, Table, Row},
    Terminal,
};
use std::io;

use arbitrage2::strategy::opportunity_queue::OpportunityConsumer;
use arbitrage2::strategy::types::ArbitrageOpportunity;

type DynError = Box<dyn Error + Send + Sync>;

#[derive(Clone, Debug)]
struct RemovedOpportunity {
    ticker: String,
    confidence_score: u8,
    reason: String,
}

struct AppState {
    opportunities: BTreeMap<String, ArbitrageOpportunity>,
    opportunity_consumer: OpportunityConsumer,
    removed_opportunities: VecDeque<RemovedOpportunity>,
    should_quit: bool,
    scroll_offset: usize,
    redis_conn: redis::aio::MultiplexedConnection,
}

impl AppState {
    fn new(opportunity_consumer: OpportunityConsumer, redis_conn: redis::aio::MultiplexedConnection) -> Self {
        Self {
            opportunities: BTreeMap::new(),
            opportunity_consumer,
            removed_opportunities: VecDeque::new(),
            should_quit: false,
            scroll_offset: 0,
            redis_conn,
        }
    }

    async fn update_from_redis(&mut self) {
        // Scan Redis for market data and detect opportunities
        // This is the legacy mode that the dashboard used before
        use arbitrage2::strategy::scanner::OpportunityScanner;
        use redis::AsyncCommands;
        
        // Get all ticker keys from Redis
        let pattern = "*:tickers:*";
        let keys: Vec<String> = match self.redis_conn.keys(pattern).await {
            Ok(k) => k,
            Err(_) => return,
        };
        
        // Group by symbol - store (exchange, bid, ask, funding_rate)
        let mut symbol_data: std::collections::HashMap<String, Vec<(String, f64, f64, f64)>> = std::collections::HashMap::new();
        
        // Also create a lookup map for quick access: (symbol, exchange) -> (bid, ask)
        let mut exchange_prices: std::collections::HashMap<(String, String), (f64, f64)> = std::collections::HashMap::new();
        
        for key in keys {
            // Parse key: "exchange:type:tickers:symbol"
            let parts: Vec<&str> = key.split(':').collect();
            if parts.len() < 4 {
                continue;
            }
            
            let exchange = parts[0];
            let symbol_raw = parts[parts.len() - 1];
            let symbol = arbitrage2::exchange_parser::normalize_symbol(symbol_raw);
            
            // Get ticker data
            let value: String = match self.redis_conn.get(&key).await {
                Ok(v) => v,
                Err(_) => continue,
            };
            
            // Parse JSON
            let json: serde_json::Value = match serde_json::from_str(&value) {
                Ok(j) => j,
                Err(_) => continue,
            };
            
            // Get parser and extract prices
            let parser = arbitrage2::exchange_parser::get_parser(exchange);
            let bid_str = match parser.parse_bid(&json) {
                Some(b) => b,
                None => continue,
            };
            let ask_str = match parser.parse_ask(&json) {
                Some(a) => a,
                None => continue,
            };
            
            let bid = match arbitrage2::exchange_parser::parse_price_simd(&bid_str) {
                Some(b) => b,
                None => continue,
            };
            let ask = match arbitrage2::exchange_parser::parse_price_simd(&ask_str) {
                Some(a) => a,
                None => continue,
            };
            
            if bid <= 0.0 || ask <= 0.0 || bid >= ask {
                continue;
            }
            
            // Get funding rate from separate Redis key
            // Different exchanges use different key patterns
            let funding_key = match exchange {
                "bybit" => format!("bybit:linear:funding:{}", symbol_raw),
                "okx" => format!("okx:usdt:funding:{}", symbol_raw),
                "bitget" => format!("bitget:usdt:funding:{}", symbol_raw),
                "gateio" => format!("gateio:usdt:funding:{}", symbol_raw),
                "kucoin" => format!("kucoin:usdt:funding:{}", symbol_raw),
                _ => format!("{}:usdt:funding:{}", exchange, symbol_raw),
            };
            
            let funding_rate: f64 = match self.redis_conn.get(&funding_key).await {
                Ok(rate_str) => {
                    let rate_str: String = rate_str;
                    
                    // Try to parse as JSON first (some exchanges store JSON objects)
                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&rate_str) {
                        // Try different field names used by different exchanges
                        json.get("fundingRate")
                            .or_else(|| json.get("funding_rate"))
                            .or_else(|| json.get("data").and_then(|d| d.get("fundingRate")))
                            .or_else(|| json.get("data").and_then(|d| d.get("funding_rate")))
                            .and_then(|v| {
                                // Try as string first, then as number
                                v.as_str().and_then(|s| s.parse::<f64>().ok())
                                    .or_else(|| v.as_f64())
                            })
                            .unwrap_or(0.0)
                    } else {
                        // Try direct string parse
                        rate_str.parse::<f64>().unwrap_or(0.0)
                    }
                },
                Err(_) => 0.0,
            };
            
            symbol_data.entry(symbol.clone()).or_insert_with(Vec::new).push((exchange.to_string(), bid, ask, funding_rate));
            exchange_prices.insert((symbol, exchange.to_string()), (bid, ask));
        }
        
        // Detect opportunities
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        
        let mut new_opportunities = BTreeMap::new();
        
        for (symbol, exchanges) in symbol_data {
            if exchanges.len() < 2 {
                continue;
            }
            
            // Check all pairs
            for i in 0..exchanges.len() {
                for j in (i + 1)..exchanges.len() {
                    let (ex1, bid1, ask1, funding1) = &exchanges[i];
                    let (ex2, bid2, ask2, funding2) = &exchanges[j];
                    
                    // Check both directions
                    // Direction 1: Long on ex1 (buy at ask1), Short on ex2 (sell at bid2)
                    let spread1 = OpportunityScanner::calculate_spread_bps(*ask1, *bid2);
                    let funding_delta1 = (funding2 - funding1).abs();
                    
                    // Direction 2: Long on ex2 (buy at ask2), Short on ex1 (sell at bid1)
                    let spread2 = OpportunityScanner::calculate_spread_bps(*ask2, *bid1);
                    let funding_delta2 = (funding1 - funding2).abs();
                    
                    // Calculate confidence scores
                    let confidence1 = ((spread1 / 50.0).min(1.0) * 50.0 + (funding_delta1 / 0.01).min(1.0) * 30.0 + 20.0) as u8;
                    let confidence2 = ((spread2 / 50.0).min(1.0) * 50.0 + (funding_delta2 / 0.01).min(1.0) * 30.0 + 20.0) as u8;
                    
                    // Only show opportunities with spread > 10 bps AND confidence >= 70
                    if spread1 > 10.0 && confidence1 >= 70 {
                        let opp = ArbitrageOpportunity {
                            symbol: symbol.clone(),
                            long_exchange: ex1.clone(),
                            short_exchange: ex2.clone(),
                            long_price: *ask1,
                            short_price: *bid2,
                            spread_bps: spread1,
                            funding_delta_8h: funding_delta1,
                            confidence_score: confidence1,
                            projected_profit_usd: (spread1 / 10000.0) * 1000.0,
                            projected_profit_after_slippage: spread1 - 20.0,
                            metrics: arbitrage2::strategy::types::ConfluenceMetrics {
                                funding_delta: funding_delta1,
                                funding_delta_projected: funding_delta1,
                                obi_ratio: 0.0,
                                oi_current: 0.0,
                                oi_24h_avg: 0.0,
                                vwap_deviation: 0.0,
                                atr: 0.0,
                                atr_trend: false,
                                liquidation_cluster_distance: 0.0,
                                hard_constraints: arbitrage2::strategy::types::HardConstraints {
                                    order_book_depth_sufficient: true,
                                    exchange_latency_ok: true,
                                    funding_delta_substantial: funding_delta1 >= 0.0001,
                                },
                            },
                            order_book_depth_long: 15000.0,
                            order_book_depth_short: 15000.0,
                            timestamp: Some(now),
                        };
                        // Store with additional bid/ask info in the key for later lookup
                        let key = format!("{}:{}:{}:{}:{}:{}:{}", symbol, ex1, ex2, bid1, ask1, bid2, ask2);
                        new_opportunities.insert(key, opp);
                    }
                    
                    if spread2 > 10.0 && confidence2 >= 70 {
                        let opp = ArbitrageOpportunity {
                            symbol: symbol.clone(),
                            long_exchange: ex2.clone(),
                            short_exchange: ex1.clone(),
                            long_price: *ask2,
                            short_price: *bid1,
                            spread_bps: spread2,
                            funding_delta_8h: funding_delta2,
                            confidence_score: confidence2,
                            projected_profit_usd: (spread2 / 10000.0) * 1000.0,
                            projected_profit_after_slippage: spread2 - 20.0,
                            metrics: arbitrage2::strategy::types::ConfluenceMetrics {
                                funding_delta: funding_delta2,
                                funding_delta_projected: funding_delta2,
                                obi_ratio: 0.0,
                                oi_current: 0.0,
                                oi_24h_avg: 0.0,
                                vwap_deviation: 0.0,
                                atr: 0.0,
                                atr_trend: false,
                                liquidation_cluster_distance: 0.0,
                                hard_constraints: arbitrage2::strategy::types::HardConstraints {
                                    order_book_depth_sufficient: true,
                                    exchange_latency_ok: true,
                                    funding_delta_substantial: funding_delta2 >= 0.0001,
                                },
                            },
                            order_book_depth_long: 15000.0,
                            order_book_depth_short: 15000.0,
                            timestamp: Some(now),
                        };
                        let key = format!("{}:{}:{}:{}:{}:{}:{}", symbol, ex2, ex1, bid2, ask2, bid1, ask1);
                        new_opportunities.insert(key, opp);
                    }
                }
            }
        }
        
        // Track removed opportunities
        for (key, old_opp) in &self.opportunities {
            if !new_opportunities.contains_key(key) {
                if self.removed_opportunities.len() >= 10 {
                    self.removed_opportunities.pop_front();
                }
                self.removed_opportunities.push_back(RemovedOpportunity {
                    ticker: old_opp.symbol.clone(),
                    confidence_score: old_opp.confidence_score,
                    reason: "No longer detected".to_string(),
                });
            }
        }
        
        self.opportunities = new_opportunities;
    }

    fn update_from_queue(&mut self) {
        // Pop batch of opportunities (up to 100)
        let batch = self.opportunity_consumer.pop_batch(100);
        
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        
        // Track which opportunities were removed
        for (symbol, old_opp) in &self.opportunities {
            if !batch.iter().any(|o| o.symbol == *symbol) {
                // Opportunity removed - determine why
                let reason = "No longer detected".to_string();
                
                // Keep only last 10 removals
                if self.removed_opportunities.len() >= 10 {
                    self.removed_opportunities.pop_front();
                }
                
                self.removed_opportunities.push_back(RemovedOpportunity {
                    ticker: symbol.clone(),
                    confidence_score: old_opp.confidence_score,
                    reason,
                });
            }
        }
        
        // Update opportunities map
        self.opportunities.clear();
        for opp in batch {
            // Filter stale opportunities (> 5 seconds old)
            if let Some(ts) = opp.timestamp {
                if now - ts > 5 {
                    // Add to removed opportunities with staleness reason
                    if self.removed_opportunities.len() >= 10 {
                        self.removed_opportunities.pop_front();
                    }
                    self.removed_opportunities.push_back(RemovedOpportunity {
                        ticker: opp.symbol.clone(),
                        confidence_score: opp.confidence_score,
                        reason: format!("Stale ({}s old)", now - ts),
                    });
                    continue;
                }
            }
            self.opportunities.insert(opp.symbol.clone(), opp);
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), DynError> {
    eprintln!("[DASHBOARD] Starting in Redis polling mode");
    eprintln!("[DASHBOARD] Polling Redis for opportunities every 100ms");
    
    // Connect to Redis
    let redis_client = redis::Client::open("redis://127.0.0.1:6379")?;
    let redis_conn = redis_client.get_multiplexed_tokio_connection().await?;
    
    // Create a local queue for demonstration (in production, this would be shared)
    use arbitrage2::strategy::opportunity_queue::OpportunityQueue;
    let queue = OpportunityQueue::new();
    let opportunity_consumer = queue.consumer();
    
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app_state = AppState::new(opportunity_consumer, redis_conn);
    let mut last_update = std::time::Instant::now();
    let update_interval = Duration::from_millis(100);

    loop {
        // Handle events with timeout
        if event::poll(Duration::from_millis(16))? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => {
                        app_state.should_quit = true;
                    }
                    KeyCode::Up => {
                        if app_state.scroll_offset > 0 {
                            app_state.scroll_offset -= 1;
                        }
                    }
                    KeyCode::Down => {
                        if app_state.scroll_offset < app_state.opportunities.len().saturating_sub(1) {
                            app_state.scroll_offset += 1;
                        }
                    }
                    KeyCode::PageUp => {
                        app_state.scroll_offset = app_state.scroll_offset.saturating_sub(10);
                    }
                    KeyCode::PageDown => {
                        app_state.scroll_offset = (app_state.scroll_offset + 10).min(
                            app_state.opportunities.len().saturating_sub(1)
                        );
                    }
                    KeyCode::Home => {
                        app_state.scroll_offset = 0;
                    }
                    KeyCode::End => {
                        app_state.scroll_offset = app_state.opportunities.len().saturating_sub(1);
                    }
                    _ => {}
                }
            }
        }

        // Task 5.3.4: Update main loop to call update_from_redis()
        // Update data from Redis periodically (100ms interval)
        if last_update.elapsed() >= update_interval {
            app_state.update_from_redis().await;
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
            Constraint::Length(4),      // Header
            Constraint::Min(8),         // Opportunities table
            Constraint::Length(12),     // Removed opportunities (max 10 + 2 for borders)
            Constraint::Length(2)       // Footer
        ])
        .split(f.size());

    // Header
    let header = ratatui::widgets::Paragraph::new("SPREAD ARBITRAGE DASHBOARD (STREAMING MODE)")
        .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
        .block(Block::default().borders(Borders::ALL).title("Live Opportunities from Queue"));
    f.render_widget(header, chunks[0]);

    // Get visible rows based on scroll offset and available height
    let table_height = chunks[1].height as usize;
    let mut sorted_opps: Vec<_> = app.opportunities.values().collect();
    sorted_opps.sort_by(|a, b| b.spread_bps.partial_cmp(&a.spread_bps).unwrap_or(std::cmp::Ordering::Equal));
    
    let visible_rows: Vec<_> = sorted_opps
        .iter()
        .skip(app.scroll_offset)
        .take(table_height.saturating_sub(2))
        .collect();

    // Opportunities table
    let rows: Vec<Row> = visible_rows
        .iter()
        .map(|opp| {
            let spread_color = if opp.spread_bps > 20.0 {
                Color::Green
            } else if opp.spread_bps > 10.0 {
                Color::Yellow
            } else {
                Color::White
            };

            // Calculate how old this opportunity is
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let age_secs = opp.timestamp.map(|ts| now.saturating_sub(ts)).unwrap_or(0);
            let age_color = if age_secs < 5 {
                Color::Green  // Fresh (< 5 seconds)
            } else if age_secs < 15 {
                Color::Yellow // Stale (5-15 seconds)
            } else {
                Color::Red    // Very stale (> 15 seconds)
            };
            let age_str = if age_secs < 60 {
                format!("{}s", age_secs)
            } else {
                format!("{}m", age_secs / 60)
            };

            // Smart decimal formatting based on price magnitude
            let format_price = |price: f64| -> String {
                if price >= 100.0 {
                    format!("${:.2}", price)  // $100+ -> 2 decimals
                } else if price >= 10.0 {
                    format!("${:.3}", price)  // $10-100 -> 3 decimals
                } else if price >= 1.0 {
                    format!("${:.4}", price)  // $1-10 -> 4 decimals
                } else if price >= 0.01 {
                    format!("${:.6}", price)  // $0.01-1 -> 6 decimals
                } else {
                    format!("${:.9}", price)  // <$0.01 -> 9 decimals
                }
            };
            
            // Extract bid/ask from the key (format: symbol:ex1:ex2:bid1:ask1:bid2:ask2)
            // Parse the key to get the actual bid/ask values
            let key_parts: Vec<&str> = app.opportunities.iter()
                .find(|(_, v)| v.symbol == opp.symbol && v.long_exchange == opp.long_exchange && v.short_exchange == opp.short_exchange)
                .map(|(k, _)| k.as_str())
                .unwrap_or("")
                .split(':')
                .collect();
            
            let (long_bid, long_ask, short_bid, short_ask) = if key_parts.len() >= 7 {
                (
                    key_parts[3].parse::<f64>().unwrap_or(opp.long_price * 0.9995),
                    key_parts[4].parse::<f64>().unwrap_or(opp.long_price),
                    key_parts[5].parse::<f64>().unwrap_or(opp.short_price),
                    key_parts[6].parse::<f64>().unwrap_or(opp.short_price * 1.0005),
                )
            } else {
                // Fallback to approximation
                (opp.long_price * 0.9995, opp.long_price, opp.short_price, opp.short_price * 1.0005)
            };

            Row::new(vec![
                Span::raw(&opp.symbol),
                Span::styled(
                    format!("{:.2}bps", opp.spread_bps),
                    Style::default().fg(spread_color),
                ),
                Span::styled(
                    format!("{}", opp.confidence_score),
                    Style::default().fg(if opp.confidence_score > 70 { Color::Green } else if opp.confidence_score > 50 { Color::Yellow } else { Color::White }),
                ),
                Span::raw(format!("{:.6}%", opp.funding_delta_8h * 100.0)),
                Span::styled(age_str, Style::default().fg(age_color)),
                Span::raw(&opp.long_exchange),
                Span::raw(format_price(long_bid)),
                Span::raw(format_price(long_ask)),
                Span::raw(&opp.short_exchange),
                Span::raw(format_price(short_bid)),
                Span::raw(format_price(short_ask)),
                Span::raw(format!("${:.2}", opp.projected_profit_usd)),
            ])
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(14),  // Symbol
            Constraint::Length(12),  // Spread
            Constraint::Length(6),   // Score
            Constraint::Length(10),  // Fund.Δ
            Constraint::Length(6),   // Age
            Constraint::Length(10),  // Long Exchange
            Constraint::Length(13),  // Long Bid
            Constraint::Length(13),  // Long Ask
            Constraint::Length(10),  // Short Exchange
            Constraint::Length(13),  // Short Bid
            Constraint::Length(13),  // Short Ask
            Constraint::Length(10),  // Profit
        ],
    )
    .header(
        Row::new(vec![
            Span::styled("Symbol", Style::default().add_modifier(Modifier::BOLD)),
            Span::styled("Spread", Style::default().add_modifier(Modifier::BOLD)),
            Span::styled("Score", Style::default().add_modifier(Modifier::BOLD)),
            Span::styled("Fund.Δ", Style::default().add_modifier(Modifier::BOLD)),
            Span::styled("Age", Style::default().add_modifier(Modifier::BOLD)),
            Span::styled("Long Ex", Style::default().add_modifier(Modifier::BOLD)),
            Span::styled("L.Bid", Style::default().add_modifier(Modifier::BOLD)),
            Span::styled("L.Ask", Style::default().add_modifier(Modifier::BOLD)),
            Span::styled("Short Ex", Style::default().add_modifier(Modifier::BOLD)),
            Span::styled("S.Bid", Style::default().add_modifier(Modifier::BOLD)),
            Span::styled("S.Ask", Style::default().add_modifier(Modifier::BOLD)),
            Span::styled("Profit", Style::default().add_modifier(Modifier::BOLD)),
        ])
        .style(Style::default().fg(Color::Cyan)),
    )
    .block(Block::default().borders(Borders::ALL).title(format!(
        "Opportunities ({} total) - Scroll: {}/{}",
        app.opportunities.len(),
        app.scroll_offset + 1,
        app.opportunities.len()
    )))
    .highlight_style(Style::default().bg(Color::DarkGray));

    f.render_widget(table, chunks[1]);

    // Removed opportunities section
    let removed_rows: Vec<Row> = app.removed_opportunities
        .iter()
        .map(|removed| {
            Row::new(vec![
                Span::raw(removed.ticker.clone()),
                Span::raw(format!("{}", removed.confidence_score)),
                Span::styled(
                    removed.reason.clone(),
                    Style::default().fg(Color::Yellow)
                ),
            ])
        })
        .collect();

    let removed_table = Table::new(
        removed_rows,
        [
            Constraint::Length(12),
            Constraint::Length(6),
            Constraint::Min(30),
        ],
    )
    .header(
        Row::new(vec![
            Span::styled("Symbol", Style::default().add_modifier(Modifier::BOLD).fg(Color::Cyan)),
            Span::styled("Conf", Style::default().add_modifier(Modifier::BOLD).fg(Color::Cyan)),
            Span::styled("Removal Reason", Style::default().add_modifier(Modifier::BOLD).fg(Color::Cyan)),
        ])
    )
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!("Recently Removed ({} total)", app.removed_opportunities.len()))
            .style(Style::default().fg(Color::DarkGray))
    );

    f.render_widget(removed_table, chunks[2]);

    // Footer with controls
    let footer_text = "↑↓: Scroll | PgUp/PgDn: Page | Home/End: Jump | q: Quit | Streaming Mode: 100ms updates";
    let footer = ratatui::widgets::Paragraph::new(footer_text)
        .style(Style::default().fg(Color::Gray))
        .block(Block::default().borders(Borders::TOP));
    f.render_widget(footer, chunks[3]);
}
