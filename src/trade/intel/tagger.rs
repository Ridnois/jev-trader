//! Free, local pre-processing before any Jev call: which Hyperliquid coin a
//! headline is about, whether it is noise, whether it repeats a story already
//! analyzed. v1 sent everything to Jev: 44% of its calls were on headlines about
//! coins it could not trade.

use std::collections::{HashMap, HashSet};

/// Common names → Hyperliquid perp names. Tickers are matched separately against
/// the live perp universe, so this only needs words a headline uses instead of a ticker.
const ALIASES: &[(&str, &str)] = &[
    ("bitcoin", "BTC"),
    ("btc", "BTC"),
    ("ethereum", "ETH"),
    ("ether", "ETH"),
    ("solana", "SOL"),
    ("xrp", "XRP"),
    ("ripple", "XRP"),
    ("dogecoin", "DOGE"),
    ("hyperliquid", "HYPE"),
    ("zcash", "ZEC"),
    ("sui", "SUI"),
    ("cardano", "ADA"),
    ("chainlink", "LINK"),
    ("avalanche", "AVAX"),
    ("toncoin", "TON"),
    ("tron", "TRX"),
    ("litecoin", "LTC"),
    ("polkadot", "DOT"),
    ("aptos", "APT"),
    ("arbitrum", "ARB"),
    ("optimism", "OP"),
    ("pepe", "kPEPE"),
    ("shiba inu", "kSHIB"),
    ("bonk", "kBONK"),
    ("ethena", "ENA"),
    ("uniswap", "UNI"),
    ("aave", "AAVE"),
    ("bnb", "BNB"),
    ("binance coin", "BNB"),
    ("worldcoin", "WLD"),
    ("pump.fun", "PUMP"),
    ("fartcoin", "FARTCOIN"),
    ("near protocol", "NEAR"),
    ("monero", "XMR"),
    ("stellar", "XLM"),
    ("hedera", "HBAR"),
    ("bitcoin cash", "BCH"),
    ("ethereum classic", "ETC"),
    ("filecoin", "FIL"),
    ("injective", "INJ"),
    ("celestia", "TIA"),
    ("jupiter", "JUP"),
    ("ondo", "ONDO"),
    ("pendle", "PENDLE"),
    ("render", "RENDER"),
    ("bittensor", "TAO"),
    ("virtuals", "VIRTUAL"),
    ("berachain", "BERA"),
    ("polygon", "POL"),
    ("kaspa", "KAS"),
    ("sei", "SEI"),
    ("mantle", "MNT"),
    ("trump coin", "TRUMP"),
];

/// Tickers that are also ordinary words or acronyms in headlines.
const TICKER_STOP: &[&str] = &[
    "THE", "FOR", "AND", "ONE", "NOW", "NEW", "ALL", "TOP", "BIG", "WAR", "CEO", "SEC", "ETF", "USD", "USA", "FED", "CPI", "GDP", "IPO", "API", "NFT", "DAO", "DEX", "CEX", "AI", "US", "UK", "EU", "IT", "ME", "MY", "GO", "IN", "ON", "OR", "BY", "AT", "TO", "IS", "BE", "DO", "SO", "UP", "OP", "ARE", "HAS", "WIN", "LAW", "RUN", "SUN", "MOVE", "GAS", "PEOPLE", "SAFE", "JUST", "BOME", "TURBO", "HOOK", "MEW", "NOT", "ZRO", "FUN", "OM", "ID", "NIL", "LIT", "PUMP", "S", "W", "ME", "BLAST", "CAKE", "SPX", "TST",
];

const MARKET_WORDS: &[&str] = &[
    "crypto", "cryptocurrency", "cryptocurrencies", "digital asset", "stablecoin", "altcoin", "defi", "etf", "sec ", "federal reserve", "fed ", "fomc",
    "rate cut", "rate hike", "inflation", "cpi", "tariff", "treasury", "liquidation", "risk-off", "risk off", "market crash", "nasdaq", "powell",
];

/// Strong noise markers: commentary, explainers, promotions. Dropped before Jev.
const NOISE: &[&str] = &[
    "price prediction", "price analysis", "price forecast", "technical analysis", "how to ", "what is ", "explained", "podcast", "newsletter", "sponsored",
    "should you", "could reach", "can reach", "will reach", "might ", "here's why", "here is why", "top 5", "top 10", "best crypto", "to buy now", "weekly recap",
    "this week in", "daily recap", "presale", "guide to", "review:", "opinion", "op-ed", "could double", "could soar", "could explode", "could rally",
    "could 10x", "prediction market",
];

/// Price-move verbs. With a number or a level in the title, the headline reports
/// a move that already happened (a recap), not news that could cause one.
const MOVE_WORDS: &[&str] = &[
    "drops", "drop ", "falls", "fall ", "slides", "slide ", "dips", "plunges", "tumbles", "sinks", "crashes", "slumps", "slips", "rises", "jumps", "surges",
    "soars", "rallies", "climbs", "spikes", "rebounds", "hits", "breaks below", "breaks above", "tops", "lows", "highs", "liquidated", "liquidation", "erases", "extends losses", "extends gains",
];

const HACK_WORDS: &[&str] = &["hack", "exploit", "drain", "stolen", "theft", "breach", "attack", "vulnerab", "compromis", "rug pull", "rugpull"];

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Tag {
    /// Hyperliquid perps named, the most likely subject first.
    pub coins: Vec<String>,
    /// About the crypto market or macro rather than (only) one coin.
    pub market: bool,
    pub noise: bool,
    /// Reports a price move that already happened.
    pub recap: bool,
    pub hack_words: bool,
}

pub struct Tagger {
    /// Upper-case ticker → Hyperliquid perp name (PEPE → kPEPE).
    tickers: HashMap<String, String>,
}

impl Tagger {
    /// `perps`: every listed Hyperliquid perp name.
    pub fn new(perps: &[String]) -> Self {
        let mut tickers = HashMap::new();
        for p in perps {
            tickers.insert(p.to_uppercase(), p.clone());
            if let Some(rest) = p.strip_prefix('k')
                && rest.chars().all(|c| c.is_ascii_uppercase())
            {
                tickers.insert(rest.to_string(), p.clone());
            }
        }
        Self { tickers }
    }

    pub fn has_perp(&self, coin: &str) -> bool {
        self.tickers.values().any(|c| c == coin)
    }

    pub fn tag(&self, title: &str, summary: &str) -> Tag {
        let mut coins: Vec<String> = Vec::new();
        let push = |c: &str, coins: &mut Vec<String>| {
            if self.has_perp(c) && !coins.iter().any(|x| x == c) {
                coins.push(c.to_string());
            }
        };
        let lower_title = title.to_lowercase();
        // 1. Explicit tickers: "(ZEC)", "$ZEC".
        for t in explicit_tickers(title) {
            if let Some(c) = self.tickers.get(&t) {
                push(&c.clone(), &mut coins);
            }
        }
        // 2. Names in the title, by position.
        let mut by_pos: Vec<(usize, &str)> = ALIASES.iter().filter_map(|(w, c)| find_word(&lower_title, w).map(|i| (i, *c))).collect();
        by_pos.sort();
        for (_, c) in by_pos {
            push(c, &mut coins);
        }
        // 3. Bare upper-case tickers in the title (≥ 3 letters, not common words).
        for w in title.split(|c: char| !c.is_ascii_alphanumeric()) {
            if w.len() >= 3 && w.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()) && w.chars().any(|c| c.is_ascii_uppercase()) && !TICKER_STOP.contains(&w)
                && let Some(c) = self.tickers.get(w)
            {
                push(&c.clone(), &mut coins);
            }
        }
        // 4. Only if the title named nothing: names in the summary.
        if coins.is_empty() {
            let ls = summary.to_lowercase();
            let mut by_pos: Vec<(usize, &str)> = ALIASES.iter().filter_map(|(w, c)| find_word(&ls, w).map(|i| (i, *c))).collect();
            by_pos.sort();
            if let Some((_, c)) = by_pos.first() {
                push(c, &mut coins);
            }
        }
        let text = format!("{lower_title} {}", summary.to_lowercase());
        let market = MARKET_WORDS.iter().any(|w| text.contains(w));
        let noise = NOISE.iter().any(|w| lower_title.contains(w)) || title.trim_end().ends_with('?');
        let hack_words = HACK_WORDS.iter().any(|w| lower_title.contains(w));
        let numeric = lower_title.contains('%') || find_word(&lower_title, "lows").is_some() || find_word(&lower_title, "highs").is_some() || has_price(&lower_title);
        let recap = !hack_words && numeric && MOVE_WORDS.iter().any(|w| find_word(&lower_title, w.trim()).is_some());
        Tag { coins, market, noise, recap, hack_words }
    }
}

/// "$75,000" / "$1.2b" style amounts.
fn has_price(s: &str) -> bool {
    let b = s.as_bytes();
    b.windows(2).any(|w| w[0] == b'$' && w[1].is_ascii_digit())
}

/// "(ZEC)" and "$ZEC" style tickers, upper-cased.
fn explicit_tickers(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let b: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < b.len() {
        let (open, close) = match b[i] {
            '(' => (true, Some(')')),
            '$' => (true, None),
            _ => (false, None),
        };
        if open {
            let mut j = i + 1;
            let mut t = String::new();
            while j < b.len() && (b[j].is_ascii_alphanumeric()) && t.len() < 12 {
                t.push(b[j]);
                j += 1;
            }
            let closed = match close {
                Some(c) => j < b.len() && b[j] == c,
                None => true,
            };
            if closed && t.len() >= 2 && t.chars().any(|c| c.is_ascii_alphabetic()) && !t.chars().all(|c| c.is_ascii_digit()) {
                out.push(t.to_uppercase());
            }
            i = j;
        } else {
            i += 1;
        }
    }
    out
}

/// Position of `w` in `s` as a whole word (or phrase).
fn find_word(s: &str, w: &str) -> Option<usize> {
    let mut from = 0;
    while let Some(i) = s[from..].find(w) {
        let at = from + i;
        let before = s[..at].chars().next_back();
        let after = s[at + w.len()..].chars().next();
        let edge = |c: Option<char>| c.is_none_or(|c| !c.is_alphanumeric());
        if edge(before) && edge(after) {
            return Some(at);
        }
        from = at + w.len();
    }
    None
}

pub fn tokens(title: &str) -> HashSet<String> {
    const STOP: &[&str] = &["the", "a", "an", "of", "to", "in", "on", "for", "and", "as", "is", "at", "by", "with", "its", "after", "from", "says", "amid", "over"];
    title
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() > 1 && !STOP.contains(w))
        .map(str::to_string)
        .collect()
}

pub fn jaccard(a: &HashSet<String>, b: &HashSet<String>) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    a.intersection(b).count() as f64 / a.union(b).count() as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tagger() -> Tagger {
        Tagger::new(&["BTC", "ETH", "SOL", "ZEC", "HYPE", "kPEPE", "SUI", "PREDICT", "ENA"].map(String::from))
    }

    #[test]
    fn tags_names_tickers_and_market() {
        let t = tagger();
        assert_eq!(t.tag("SEC approves spot Solana ETFs", "").coins, vec!["SOL"]);
        assert!(t.tag("SEC approves spot Solana ETFs", "").market);
        assert_eq!(t.tag("Binance Will List Predict (PREDICT) with Seed Tag", "").coins, vec!["PREDICT"]);
        assert_eq!(t.tag("Whales rotate from $PEPE into Zcash", "").coins, vec!["kPEPE", "ZEC"]);
        assert_eq!(t.tag("ENA unlock looms as Ethena TVL falls", "").coins, vec!["ENA"]);
        // "ether" inside "together" is not Ethereum.
        assert!(t.tag("Banks band together on payments", "").coins.is_empty());
        // Summary only counts when the title names nothing.
        assert_eq!(t.tag("Fund outflows hit record", "Bitcoin products lost $1B").coins, vec!["BTC"]);
    }

    #[test]
    fn flags_noise_and_hacks() {
        let t = tagger();
        assert!(t.tag("Solana price prediction: can SOL reach $500?", "").noise);
        assert!(t.tag("Is Bitcoin about to break out?", "").noise);
        assert!(!t.tag("Hyperliquid daily volume hits record $20B", "").noise);
        assert!(t.tag("Hacker drains $40M from Cetus on Sui", "").hack_words);
        assert!(!t.tag("Hacker drains $40M from Cetus on Sui", "").recap, "a hack is news even with an amount");
        assert!(t.tag("Bitcoin price drops 2% in one hour to $74,244", "").recap);
        assert!(t.tag("Bitcoin and Ether fall to lows since August", "").recap);
        assert!(!t.tag("SEC approves spot Solana ETFs", "").recap);
        assert!(!t.tag("Circle froze $1.1B in USDC linked to Binance probe", "").recap);
        assert!(!t.tag("Binance Restores HYPE as Data Reveals Billions in Reserve Outflows", "").recap, "\"lows\" inside a word");
    }

    #[test]
    fn jaccard_matches_rewrites() {
        let a = tokens("Bitcoin falls below $80,000 as stocks slide");
        let b = tokens("Bitcoin falls below $80,000 while US stocks slide");
        assert!(jaccard(&a, &b) > 0.5);
        assert!(jaccard(&a, &tokens("Ethereum Foundation sells 10,000 ETH")) < 0.2);
    }
}
