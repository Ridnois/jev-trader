//! 3.4: drawdown circuit breaker. Daily kill switch (goes flat, as before), plus
//! optional weekly / monthly halts and a losing-streak cooldown that block new
//! entries but never force liquidation. Corrupt ledger state fails closed.

use chrono::{DateTime, Datelike, Duration, NaiveDate, TimeZone, Utc};
use serde::{Deserialize, Serialize};

use crate::config::RiskConfig;
use crate::paper::State;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum BreakerState {
    TradingAllowed,
    Cooldown,
    Halted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Breaker {
    pub state: BreakerState,
    pub reason: String,
    pub active_until: Option<DateTime<Utc>>,
    /// Daily kill switch tripped: positions go flat (original behaviour).
    pub flatten: bool,
}

impl Breaker {
    pub fn allowed() -> Self {
        Self { state: BreakerState::TradingAllowed, reason: String::new(), active_until: None, flatten: false }
    }

    pub fn blocks_entries(&self) -> bool {
        self.state != BreakerState::TradingAllowed
    }
}

fn midnight(d: NaiveDate) -> DateTime<Utc> {
    Utc.from_utc_datetime(&d.and_hms_opt(0, 0, 0).expect("midnight"))
}

pub fn week_start(d: NaiveDate) -> NaiveDate {
    d - Duration::days(d.weekday().num_days_from_monday() as i64)
}

pub fn month_start(d: NaiveDate) -> NaiveDate {
    d.with_day(1).expect("day 1")
}

fn next_month(d: NaiveDate) -> NaiveDate {
    let (y, m) = if d.month() == 12 { (d.year() + 1, 1) } else { (d.year(), d.month() + 1) };
    NaiveDate::from_ymd_opt(y, m, 1).expect("valid month")
}

/// Rolls the day/week/month baselines and evaluates every breaker rule.
/// `equity` is `None` when some held position has no verified price: then nothing
/// rolls and no loss rule is evaluated (0.4), the caller puts the account in REVIEW.
pub fn update(state: &mut State, equity: Option<f64>, now: DateTime<Utc>, risk: &RiskConfig) -> Breaker {
    let mut worst = Breaker::allowed();
    let mut raise = |b: Breaker| {
        if (b.state, b.flatten) > (worst.state, worst.flatten) {
            worst = b;
        }
    };

    if let Err(e) = state.validate() {
        raise(Breaker { state: BreakerState::Halted, reason: format!("corrupt state: {e}"), active_until: None, flatten: false });
        return worst;
    }
    let today = now.date_naive();

    if let Some(eq) = equity.filter(|e| e.is_finite()) {
        if today != state.day {
            state.day = today;
            state.day_start_equity = eq;
        }
        let ws = week_start(today);
        if state.week_start != Some(ws) {
            state.week_start = Some(ws);
            state.week_start_equity = Some(eq);
        }
        let ms = month_start(today);
        if state.month_start != Some(ms) {
            state.month_start = Some(ms);
            state.month_start_equity = Some(eq);
        }
        if eq < state.day_start_equity * (1.0 - risk.daily_loss_limit_pct) {
            state.halted_on = Some(today);
        }
        if let (Some(lim), Some(start)) = (risk.weekly_loss_limit_pct, state.week_start_equity)
            && eq < start * (1.0 - lim)
        {
            state.halted_week = Some(ws);
        }
        if let (Some(lim), Some(start)) = (risk.monthly_loss_limit_pct, state.month_start_equity)
            && eq < start * (1.0 - lim)
        {
            state.halted_month = Some(ms);
        }
    }

    if state.halted_on == Some(today) {
        raise(Breaker {
            state: BreakerState::Halted,
            reason: "daily loss limit".into(),
            active_until: Some(midnight(today + Duration::days(1))),
            flatten: true,
        });
    }
    if state.halted_week == Some(week_start(today)) {
        raise(Breaker {
            state: BreakerState::Halted,
            reason: "weekly halt".into(),
            active_until: Some(midnight(week_start(today) + Duration::days(7))),
            flatten: false,
        });
    }
    if state.halted_month == Some(month_start(today)) {
        raise(Breaker {
            state: BreakerState::Halted,
            reason: "monthly halt".into(),
            active_until: Some(midnight(next_month(today))),
            flatten: false,
        });
    }
    if let Some(until) = state.streak_until.filter(|u| *u > now) {
        raise(Breaker {
            state: BreakerState::Cooldown,
            reason: "losing streak".into(),
            active_until: Some(until),
            flatten: false,
        });
    }
    worst
}

#[cfg(test)]
mod tests {
    use super::*;

    fn risk() -> RiskConfig {
        toml::from_str(
            r#"max_asset_weight = 0.25
max_gross = 0.6
stop_loss_pct = 0.04
daily_loss_limit_pct = 0.03
cooldown_hours = 6.0
min_trade_usd = 10.0
rebalance_band = 0.02
weekly_loss_limit_pct = 0.05"#,
        )
        .unwrap()
    }

    #[test]
    fn daily_kill_switch_flattens_for_the_day() {
        let r = risk();
        let now = Utc::now();
        let mut s = State::new(10_000.0);
        assert!(!update(&mut s, Some(9_800.0), now, &r).blocks_entries());
        let b = update(&mut s, Some(9_600.0), now, &r);
        assert!(b.flatten && b.state == BreakerState::Halted);
        assert!(update(&mut s, Some(10_000.0), now, &r).flatten, "stays halted for the day");
    }

    #[test]
    fn weekly_halt_blocks_but_does_not_flatten() {
        let r = risk();
        let monday = Utc.with_ymd_and_hms(2026, 11, 2, 1, 0, 0).unwrap();
        let mut s = State::new(10_000.0);
        update(&mut s, Some(10_000.0), monday, &r);
        update(&mut s, Some(9_800.0), monday + Duration::days(1), &r);
        let wed = monday + Duration::days(2);
        update(&mut s, Some(9_520.0), wed, &r); // new day baseline 9520
        let b = update(&mut s, Some(9_490.0), wed, &r); // −0.3% today, −5.1% this week
        assert_eq!(b.state, BreakerState::Halted);
        assert_eq!(b.reason, "weekly halt");
        assert!(!b.flatten);
        let next_week = monday + Duration::days(7);
        assert!(!update(&mut s, Some(9_490.0), next_week, &r).blocks_entries());
    }

    #[test]
    fn unverified_equity_does_not_trip_or_roll() {
        let r = risk();
        let mut s = State::new(10_000.0);
        let b = update(&mut s, None, Utc::now() + Duration::days(3), &r);
        assert!(!b.blocks_entries());
        assert!(s.week_start.is_none());
    }

    #[test]
    fn corrupt_state_halts() {
        let r = risk();
        let mut s = State::new(10_000.0);
        s.cash = f64::INFINITY;
        assert_eq!(update(&mut s, Some(1.0), Utc::now(), &r).state, BreakerState::Halted);
    }
}
