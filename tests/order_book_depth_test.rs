use arbitrage2::strategy::types::{OrderBookDepth, PriceLevel};
use serde_json;

#[test]
fn test_price_level_creation() {
    let level = PriceLevel {
        price: 50000.0,
        quantity: 1.5,
    };
    
    assert_eq!(level.price, 50000.0);
    assert_eq!(level.quantity, 1.5);
}

#[test]
fn test_price_level_serialization() {
    let level = PriceLevel {
        price: 50000.0,
        quantity: 1.5,
    };
    
    let json = serde_json::to_string(&level).expect("Failed to serialize PriceLevel");
    assert!(json.contains("50000"));
    assert!(json.contains("1.5"));
}

#[test]
fn test_price_level_deserialization() {
    let json = r#"{"price":50000.0,"quantity":1.5}"#;
    let level: PriceLevel = serde_json::from_str(json).expect("Failed to deserialize PriceLevel");
    
    assert_eq!(level.price, 50000.0);
    assert_eq!(level.quantity, 1.5);
}

#[test]
fn test_price_level_round_trip() {
    let original = PriceLevel {
        price: 42000.75,
        quantity: 2.345,
    };
    
    let json = serde_json::to_string(&original).expect("Failed to serialize");
    let deserialized: PriceLevel = serde_json::from_str(&json).expect("Failed to deserialize");
    
    assert_eq!(original.price, deserialized.price);
    assert_eq!(original.quantity, deserialized.quantity);
}

#[test]
fn test_order_book_depth_creation() {
    let bids = vec![
        PriceLevel { price: 50000.0, quantity: 1.0 },
        PriceLevel { price: 49999.0, quantity: 2.0 },
    ];
    
    let asks = vec![
        PriceLevel { price: 50001.0, quantity: 1.5 },
        PriceLevel { price: 50002.0, quantity: 2.5 },
    ];
    
    let depth = OrderBookDepth {
        bids: bids.clone(),
        asks: asks.clone(),
        timestamp: 1234567890,
    };
    
    assert_eq!(depth.bids.len(), 2);
    assert_eq!(depth.asks.len(), 2);
    assert_eq!(depth.timestamp, 1234567890);
    assert_eq!(depth.bids[0].price, 50000.0);
    assert_eq!(depth.asks[0].price, 50001.0);
}

#[test]
fn test_order_book_depth_serialization() {
    let depth = OrderBookDepth {
        bids: vec![
            PriceLevel { price: 50000.0, quantity: 1.0 },
        ],
        asks: vec![
            PriceLevel { price: 50001.0, quantity: 1.5 },
        ],
        timestamp: 1234567890,
    };
    
    let json = serde_json::to_string(&depth).expect("Failed to serialize OrderBookDepth");
    assert!(json.contains("bids"));
    assert!(json.contains("asks"));
    assert!(json.contains("timestamp"));
    assert!(json.contains("50000"));
    assert!(json.contains("50001"));
    assert!(json.contains("1234567890"));
}

#[test]
fn test_order_book_depth_deserialization() {
    let json = r#"{
        "bids": [
            {"price": 50000.0, "quantity": 1.0},
            {"price": 49999.0, "quantity": 2.0}
        ],
        "asks": [
            {"price": 50001.0, "quantity": 1.5},
            {"price": 50002.0, "quantity": 2.5}
        ],
        "timestamp": 1234567890
    }"#;
    
    let depth: OrderBookDepth = serde_json::from_str(json).expect("Failed to deserialize OrderBookDepth");
    
    assert_eq!(depth.bids.len(), 2);
    assert_eq!(depth.asks.len(), 2);
    assert_eq!(depth.timestamp, 1234567890);
    assert_eq!(depth.bids[0].price, 50000.0);
    assert_eq!(depth.bids[0].quantity, 1.0);
    assert_eq!(depth.asks[0].price, 50001.0);
    assert_eq!(depth.asks[0].quantity, 1.5);
}

#[test]
fn test_order_book_depth_round_trip() {
    let original = OrderBookDepth {
        bids: vec![
            PriceLevel { price: 50000.0, quantity: 1.0 },
            PriceLevel { price: 49999.0, quantity: 2.0 },
            PriceLevel { price: 49998.0, quantity: 3.0 },
        ],
        asks: vec![
            PriceLevel { price: 50001.0, quantity: 1.5 },
            PriceLevel { price: 50002.0, quantity: 2.5 },
            PriceLevel { price: 50003.0, quantity: 3.5 },
        ],
        timestamp: 1234567890,
    };
    
    let json = serde_json::to_string(&original).expect("Failed to serialize");
    let deserialized: OrderBookDepth = serde_json::from_str(&json).expect("Failed to deserialize");
    
    assert_eq!(original.bids.len(), deserialized.bids.len());
    assert_eq!(original.asks.len(), deserialized.asks.len());
    assert_eq!(original.timestamp, deserialized.timestamp);
    
    for (i, bid) in original.bids.iter().enumerate() {
        assert_eq!(bid.price, deserialized.bids[i].price);
        assert_eq!(bid.quantity, deserialized.bids[i].quantity);
    }
    
    for (i, ask) in original.asks.iter().enumerate() {
        assert_eq!(ask.price, deserialized.asks[i].price);
        assert_eq!(ask.quantity, deserialized.asks[i].quantity);
    }
}

#[test]
fn test_order_book_depth_empty_levels() {
    let depth = OrderBookDepth {
        bids: vec![],
        asks: vec![],
        timestamp: 1234567890,
    };
    
    let json = serde_json::to_string(&depth).expect("Failed to serialize");
    let deserialized: OrderBookDepth = serde_json::from_str(&json).expect("Failed to deserialize");
    
    assert_eq!(deserialized.bids.len(), 0);
    assert_eq!(deserialized.asks.len(), 0);
    assert_eq!(deserialized.timestamp, 1234567890);
}

#[test]
fn test_order_book_depth_large_numbers() {
    let depth = OrderBookDepth {
        bids: vec![
            PriceLevel { price: 99999.99, quantity: 1000.123456 },
        ],
        asks: vec![
            PriceLevel { price: 100000.01, quantity: 2000.654321 },
        ],
        timestamp: 9999999999,
    };
    
    let json = serde_json::to_string(&depth).expect("Failed to serialize");
    let deserialized: OrderBookDepth = serde_json::from_str(&json).expect("Failed to deserialize");
    
    assert_eq!(deserialized.bids[0].price, 99999.99);
    assert_eq!(deserialized.bids[0].quantity, 1000.123456);
    assert_eq!(deserialized.asks[0].price, 100000.01);
    assert_eq!(deserialized.asks[0].quantity, 2000.654321);
    assert_eq!(deserialized.timestamp, 9999999999);
}

#[test]
fn test_order_book_depth_clone() {
    let original = OrderBookDepth {
        bids: vec![
            PriceLevel { price: 50000.0, quantity: 1.0 },
        ],
        asks: vec![
            PriceLevel { price: 50001.0, quantity: 1.5 },
        ],
        timestamp: 1234567890,
    };
    
    let cloned = original.clone();
    
    assert_eq!(original.bids.len(), cloned.bids.len());
    assert_eq!(original.asks.len(), cloned.asks.len());
    assert_eq!(original.timestamp, cloned.timestamp);
    assert_eq!(original.bids[0].price, cloned.bids[0].price);
    assert_eq!(original.asks[0].price, cloned.asks[0].price);
}

#[test]
fn test_price_level_clone() {
    let original = PriceLevel {
        price: 50000.0,
        quantity: 1.5,
    };
    
    let cloned = original.clone();
    
    assert_eq!(original.price, cloned.price);
    assert_eq!(original.quantity, cloned.quantity);
}
