# Trading Monitor - Ratatui Implementation

## Overview

The trading monitor has been updated to use **ratatui** for terminal UI rendering, eliminating flickering and providing a smooth, professional real-time display.

## Key Improvements

### Before (ANSI escape codes)
- Screen cleared and reprinted every update (`\x1B[2J\x1B[1;1H`)
- Flickering and visual artifacts
- Non-blocking keyboard input was incomplete
- Manual terminal state management

### After (Ratatui)
- Double-buffered rendering (no flickering)
- Proper terminal state management
- Full keyboard event handling
- Clean, structured UI layout
- Professional appearance with borders and colors

## Architecture

### Main Components

1. **AppState** - Holds all application data
   - `metrics`: Current portfolio metrics
   - `state`: Portfolio state with active/closed trades
   - `should_quit`: Exit flag
   - `active_scroll_offset`: Scroll position for active trades
   - `exits_scroll_offset`: Scroll position for recent exits

2. **Event Loop**
   - Polls for keyboard events (16ms timeout)
   - Updates data from Redis every 1 second
   - Renders UI using ratatui
   - Handles scrolling and navigation

3. **UI Sections**
   - **Portfolio Summary**: Capital, trades, win rate, P&L, APR
   - **Active Trades Table**: Ticker, entry/current spread, unrealized P&L, exchanges
   - **Recent Exits Table**: Ticker, profit, exit reason
   - **Footer**: Keyboard controls

## Keyboard Controls

| Key | Action |
|-----|--------|
| `q` / `Esc` | Quit |
| `↑` / `↓` | Scroll active trades |
| `PgUp` / `PgDn` | Page up/down active trades |
| `Home` / `End` | Jump to start/end of active trades |
| `j` / `k` | Scroll recent exits |

## Color Coding

- **Green**: Positive P&L, high confidence
- **Red**: Negative P&L, low confidence
- **Yellow**: Medium values
- **Cyan**: Headers and titles
- **Gray**: Footer text

## Data Flow

```
Redis (strategy:portfolio:metrics, strategy:portfolio:state)
    ↓
AppState::update_from_redis() [every 1 second]
    ↓
Terminal::draw(|f| ui(f, &app_state))
    ↓
Ratatui renders:
  - Portfolio Summary
  - Active Trades Table
  - Recent Exits Table
  - Footer
```

## Running the Monitor

```bash
# Terminal 1: Start Redis
redis-server

# Terminal 2: Run strategy runner
cargo run --release

# Terminal 3: Run trading monitor
cargo run --bin trading-monitor --release
```

## Technical Details

### Ratatui Features Used

- **Terminal Management**: `enable_raw_mode()`, `EnterAlternateScreen`
- **Event Handling**: Crossterm event polling with timeout
- **Layout**: Vertical layout with constraints
- **Widgets**: Table, Paragraph, Block, Row, Span
- **Styling**: Color, Modifier (Bold), Style
- **Double Buffering**: Automatic via ratatui

### Performance

- **Update Frequency**: 1 second (configurable)
- **Event Poll Timeout**: 16ms (60 FPS)
- **Memory**: Minimal - only stores current state
- **CPU**: Low - only renders on updates

## Advantages Over Previous Implementation

1. **No Flickering**: Double-buffered rendering
2. **Responsive**: 60 FPS event polling
3. **Professional**: Clean borders, colors, alignment
4. **Scalable**: Handles large trade lists with scrolling
5. **Maintainable**: Structured code with clear separation of concerns
6. **Cross-Platform**: Works on Windows, macOS, Linux

## Future Enhancements

- Real-time price updates (not simulated)
- Configurable update frequency
- Export to CSV/JSON
- Historical charts
- Alerts for significant events
- Theme customization
