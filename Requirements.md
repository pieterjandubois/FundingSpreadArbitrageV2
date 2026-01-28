This document will be updated by the user incrementally with features that should be implemented by the agents. You should not ever attemp to change anything in here, in fact you are forbidden to do so.

We're building a algorithm that will detect arbitrage opportunities inbetween platforms (cexes and dexes) in the crypto markets.
We're focussing on the future markets and not spot.

Triggers will be a highly differentiating funding rate together with other features that will be listed in this document over time.

The general idea is that if funding rates from the same ticker differ a substantial amount between platforms we will build a delta neutral position trying to capture the evening of the spread that has built up between the long/short side in ticker price. example sol 103$ on binance and sol 100$ on bybit with a funding rate that is substantially positive. The trigger will be the high funding rate indicating a spread between future ticker prices and the goal is to capture the spread minimizing between the platforms. The trigger will also keep in mind the funding costs and fee's of opening these positions. One of the triggers to stop the trade is if funding rates are equalizing and 90% of the profit compared to the projected profit has been realized with limit orders. 

Process:

we're building in rust inless not possible otherwise we can choose other languages to achieve a certain future.

1. Setup a connection to the futures market from binance (usdt pairs only)
2. Collect all valid trading symbols from the futures market
3. subscribe using multiplexing and batching to all these symbols capturing price, funding rate and other nessecary parameters to avoid rate limits using redis-server. 
 this feature should execute the subscribing as fast as possible without achieving rate limit issues. Once data is flowing through the redis server it should be fetched at a speed we avoid rate limit issues.

DONE

4. Setup a connection to the futures market from BYBIT (usdt pairs only)
5. Collect all valid trading symbols from the futures market
6. subscribe using multiplexing and batching to all these symbols capturing price, funding rate and other nessecary parameters to avoid rate limits using redis-server. 
 this feature should execute the subscribing as fast as possible without achieving rate limit issues. Once data is flowing through the redis server it should be fetched at a speed we avoid rate limit issues.


7. Setup a connection to the futures market from kucoin (usdt pairs only)
8. Collect all valid trading symbols from the futures market
9. subscribe using multiplexing and batching to all these symbols capturing price, funding rate and other nessecary parameters to avoid rate limits using redis-server. 
 this feature should execute the subscribing as fast as possible without achieving rate limit issues. Once data is flowing through the redis server it should be fetched at a speed we avoid rate limit issues.

10. Setup a connection to the futures market from OKX 
11. Collect all valid trading symbols from the futures market (usdt pairs only)
12. subscribe using multiplexing and batching to all these symbols capturing price, funding rate and other nessecary parameters to avoid rate limits using redis-server. 
 this feature should execute the subscribing as fast as possible without achieving rate limit issues. Once data is flowing through the redis server it should be fetched at a speed we avoid rate limit issues.

 
13. Setup a connection to the futures market from BITGET
14. Collect all valid trading symbols from the futures market (usdt pairs only)
15. subscribe using multiplexing and batching to all these symbols capturing price, funding rate and other nessecary parameters to avoid rate limits using redis-server. 
 this feature should execute the subscribing as fast as possible without achieving rate limit issues. Once data is flowing through the redis server it should be fetched at a speed we avoid rate limit issues.

16. Setup a connection to the futures market from GATE
17. Collect all valid trading symbols from the futures market (usdt pairs only)
18. subscribe using multiplexing and batching to all these symbols capturing price, funding rate and other nessecary parameters to avoid rate limits using redis-server. 
 this feature should execute the subscribing as fast as possible without achieving rate limit issues. Once data is flowing through the redis server it should be fetched at a speed we avoid rate limit issues.

19. Setup a connection to the futures market from HYPERLIQUID
20. Collect all valid trading symbols from the futures market (usdt pairs only)
21. subscribe using multiplexing and batching to all these symbols capturing price, funding rate and other nessecary parameters to avoid rate limits using redis-server. 
 this feature should execute the subscribing as fast as possible without achieving rate limit issues. Once data is flowing through the redis server it should be fetched at a speed we avoid rate limit issues.

22. Setup a connection to the futures market from LIGHTER
23. Collect all valid trading symbols from the futures market (usdt pairs only)
24. subscribe using multiplexing and batching to all these symbols capturing price, funding rate and other nessecary parameters to avoid rate limits using redis-server. 
 this feature should execute the subscribing as fast as possible without achieving rate limit issues. Once data is flowing through the redis server it should be fetched at a speed we avoid rate limit issues.

25. Setup a connection to the futures market from PARADEX
26. Collect all valid trading symbols from the futures market (usdt pairs only) 
27. subscribe using multiplexing and batching to all these symbols capturing price, funding rate, orderbook depth, bid/ask and other nessecary parameters to avoid rate limits using redis-server. 
 this feature should execute the subscribing as fast as possible without achieving rate limit issues. Once data is flowing through the redis server it should be fetched at a speed we avoid rate limit issues.

Here are links you can visit for information you may need. Don't visit them all unless you need additional information:

bybit flow

while ($true) {
  $out = redis-cli -n 0 --raw SCAN 0 MATCH "bybit:linear:tickers:*" COUNT 10000
  $lines = $out -split "`n"
  $keys = $lines | Select-Object -Skip 1 | Where-Object { $_.Trim() -ne "" }

  Clear-Host
  "keys: $($keys.Count) time: $(Get-Date -Format HH:mm:ss)"

  foreach ($k in $keys) {
    "$k => $(redis-cli -n 0 GET $k)"
  }

  Start-Sleep -Seconds 2
}