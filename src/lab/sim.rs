//! The one simulator. A plan's signal at bar `i` is acted on at the open of bar
//! `i + 1`; stops are checked against each bar's high and low and fill at the stop
//! (or at the open if price gapped through), with extra stop slippage; time stops
//! fill at the first open at or past the deadline. Fees, half the spread and
//! slippage are paid on both sides, and funding prints from the archive are charged
//! while a position is open.
//!
//! Output is per UTC day, as a fraction of a fixed capital base (no compounding):
//! days are the unit every test in `gate` works on, because trades overlap and
//! cluster and are not independent observations.

use serde::Serialize;

use super::panel::Panel;
use super::rules::{self, Costs, Sizing};
use super::strategy::Plan;

#[derive(Debug, Clone)]
pub struct SimCfg {
    pub capital: f64,
    pub costs: Costs,
    pub sizing: Sizing,
    /// Extra bars between the signal and the fill (stress test). 0 = next open.
    pub entry_delay: usize,
    /// No new entries for the rest of a UTC day once closed trades have lost this
    /// share of capital. 0 = off.
    pub daily_loss_limit_pct: f64,
}

impl SimCfg {
    pub fn new(capital: f64, costs: Costs, sizing: Sizing) -> Self {
        Self { capital, costs, sizing, entry_delay: 0, daily_loss_limit_pct: 0.0 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    Time,
    Stop,
    Signal,
    End,
}

#[derive(Debug, Clone, Serialize)]
pub struct Trade {
    pub coin: usize,
    /// +1 long, −1 short.
    pub side: i8,
    pub entry_i: usize,
    pub exit_i: usize,
    pub entry_px: f64,
    pub exit_px: f64,
    pub notional: f64,
    /// After both fees and funding, dollars.
    pub pnl: f64,
    pub funding: f64,
    pub reason: Reason,
}

impl Trade {
    pub fn bps(&self) -> f64 {
        self.pnl / self.notional * 1e4
    }
}

#[derive(Debug, Clone, Default)]
pub struct Run {
    /// Mark-to-market P&L per UTC day ÷ capital; one entry per panel day.
    pub daily: Vec<f64>,
    pub trades: Vec<Trade>,
    /// Closed-trade P&L per coin, dollars.
    pub coin_pnl: Vec<f64>,
    pub fees: f64,
    pub funding: f64,
    /// Notional traded (both sides) ÷ capital.
    pub turnover: f64,
}

impl Run {
    pub fn wins(&self) -> usize {
        self.trades.iter().filter(|t| t.pnl > 0.0).count()
    }

    pub fn mean_trade_bps(&self) -> f64 {
        if self.trades.is_empty() { 0.0 } else { self.trades.iter().map(Trade::bps).sum::<f64>() / self.trades.len() as f64 }
    }

    #[cfg(test)]
    pub fn total_return(&self) -> f64 {
        self.daily.iter().sum()
    }

    /// Deepest peak-to-trough fall of the daily equity curve, as a share of capital.
    #[cfg(test)]
    pub fn max_drawdown(&self) -> f64 {
        let (mut cum, mut peak, mut dd) = (0.0f64, 0.0f64, 0.0f64);
        for r in &self.daily {
            cum += r;
            peak = peak.max(cum);
            dd = dd.max(peak - cum);
        }
        dd
    }
}

struct Pos {
    side: f64,
    qty: f64,
    entry_px: f64,
    notional: f64,
    entry_fee: f64,
    stop_px: f64,
    deadline: usize,
    entry_i: usize,
    funding: f64,
}

pub fn simulate(p: &Panel, plan: &Plan, cfg: &SimCfg) -> Run {
    let nc = p.coins.len();
    let pol = plan.policy;
    let fee = cfg.costs.fee();
    let side_bps: Vec<f64> = p.coins.iter().map(|c| cfg.costs.side_bps(c)).collect();
    let stop_extra = cfg.costs.stop_extra_bps();
    let mut pos: Vec<Option<Pos>> = (0..nc).map(|_| None).collect();
    let mut last_px = vec![f64::NAN; nc];
    let mut fund_cur: Vec<usize> = p.funding.iter().map(|f| f.partition_point(|(t, _)| *t <= p.t0)).collect();
    let mut run = Run { coin_pnl: vec![0.0; nc], daily: Vec::with_capacity(p.n_days()), ..Default::default() };
    let (mut realized, mut eq_prev) = (0.0f64, 0.0f64);
    let (mut day, mut day_closed, mut open_count, mut gross) = (usize::MAX, 0.0f64, 0usize, 0.0f64);

    // Closes `c`'s position at reference price `px`.
    let close = |pos: &mut Vec<Option<Pos>>, run: &mut Run, realized: &mut f64, day_closed: &mut f64, open_count: &mut usize, gross: &mut f64, c: usize, px: f64, i: usize, reason: Reason| {
        let Some(q) = pos[c].take() else { return };
        let extra = if reason == Reason::Stop { stop_extra } else { 0.0 };
        let fill = rules::exit_fill(px, q.side, side_bps[c] + extra);
        let exit_notional = q.qty * fill;
        let exit_fee = exit_notional * fee;
        *realized += q.side * (fill - q.entry_px) * q.qty - exit_fee;
        let pnl = q.side * (fill - q.entry_px) * q.qty - q.entry_fee - exit_fee + q.funding;
        *day_closed += pnl;
        *open_count -= 1;
        *gross -= q.notional;
        run.fees += exit_fee;
        run.turnover += exit_notional / cfg.capital;
        run.coin_pnl[c] += pnl;
        run.trades.push(Trade { coin: c, side: q.side as i8, entry_i: q.entry_i, exit_i: i, entry_px: q.entry_px, exit_px: fill, notional: q.notional, pnl, funding: q.funding, reason });
    };

    for i in 0..p.n {
        let d = p.day_of(i);
        if d != day {
            day = d;
            day_closed = 0.0;
        }
        let t_open = p.time(i);
        // Funding printed since the previous open, on positions held through it.
        if cfg.costs.funding {
            for c in 0..nc {
                let f = &p.funding[c];
                while fund_cur[c] < f.len() && f[fund_cur[c]].0 <= t_open {
                    if let Some(q) = pos[c].as_mut()
                        && last_px[c].is_finite()
                    {
                        let amt = -q.side * f[fund_cur[c]].1 * q.qty * last_px[c];
                        q.funding += amt;
                        realized += amt;
                        run.funding += amt;
                    }
                    fund_cur[c] += 1;
                }
            }
        }
        let halted = cfg.daily_loss_limit_pct > 0.0 && day_closed < -cfg.daily_loss_limit_pct * cfg.capital;
        // 1. Exits decided at the previous close, filled at this open. All of them come
        // before any entry, so the capital they free is there for this bar's entries.
        for c in 0..nc {
            let o = p.open[c][i];
            if !o.is_finite() || i == 0 {
                continue; // no bar: nothing can trade, an open position waits
            }
            if let Some((side, deadline)) = pos[c].as_ref().map(|q| (q.side as i8, q.deadline)) {
                if !plan.exit.is_empty() && plan.exit[c][i - 1] == side {
                    close(&mut pos, &mut run, &mut realized, &mut day_closed, &mut open_count, &mut gross, c, o, i, Reason::Signal);
                } else if i >= deadline {
                    if pol.extend && plan.entry[c][i - 1] == side {
                        if let Some(q) = pos[c].as_mut() {
                            q.deadline = i + pol.hold_bars;
                        }
                    } else {
                        close(&mut pos, &mut run, &mut realized, &mut day_closed, &mut open_count, &mut gross, c, o, i, Reason::Time);
                    }
                }
            }
        }
        // 2. Entries at this open.
        for c in 0..nc {
            let o = p.open[c][i];
            if !o.is_finite() || pos[c].is_some() || i <= cfg.entry_delay {
                continue;
            }
            let j = i - 1 - cfg.entry_delay;
            let sig = plan.entry[c][j];
            if sig != 0 && p.active[c][i - 1] && !halted && (pol.max_positions == 0 || open_count < pol.max_positions) {
                let usd = rules::size_usd(&cfg.sizing, cfg.capital, p.dvol[c][i - 1], gross);
                if usd > 0.0 {
                    let side = sig as f64;
                    let fill = rules::entry_fill(o, side, side_bps[c]);
                    let entry_fee = usd * fee;
                    realized -= entry_fee;
                    run.fees += entry_fee;
                    run.turnover += usd / cfg.capital;
                    open_count += 1;
                    gross += usd;
                    let stop = if pol.stop_bps > 0.0 { rules::stop_px(fill, side, pol.stop_bps) } else { f64::NAN };
                    pos[c] = Some(Pos { side, qty: usd / fill, entry_px: fill, notional: usd, entry_fee, stop_px: stop, deadline: i + pol.hold_bars, entry_i: i, funding: 0.0 });
                }
            }
        }
        // 3. Stops inside this bar (also for positions opened at its open). A stop can't
        // free capital for an entry at the same bar's open: it happens later.
        for c in 0..nc {
            let o = p.open[c][i];
            if o.is_finite() {
                let hit = pos[c].as_ref().filter(|q| q.stop_px.is_finite()).and_then(|q| {
                    if q.side > 0.0 { (p.low[c][i] <= q.stop_px).then(|| o.min(q.stop_px)) } else { (p.high[c][i] >= q.stop_px).then(|| o.max(q.stop_px)) }
                });
                if let Some(px) = hit {
                    close(&mut pos, &mut run, &mut realized, &mut day_closed, &mut open_count, &mut gross, c, px, i, Reason::Stop);
                }
            }
            if p.close[c][i].is_finite() {
                last_px[c] = p.close[c][i];
            }
        }
        let last = i + 1 == p.n;
        if last {
            // Whatever is still open closes at the last price, so every trade counts.
            for c in 0..nc {
                if pos[c].is_some() {
                    close(&mut pos, &mut run, &mut realized, &mut day_closed, &mut open_count, &mut gross, c, last_px[c], i, Reason::End);
                }
            }
        }
        if last || p.day_of(i + 1) != d {
            let unreal: f64 = (0..nc).filter_map(|c| pos[c].as_ref().map(|q| q.side * (last_px[c] - q.entry_px) * q.qty)).sum();
            let eq = realized + unreal;
            run.daily.push((eq - eq_prev) / cfg.capital);
            eq_prev = eq;
        }
    }
    run
}

#[cfg(test)]
mod tests {
    use super::super::panel::testing::*;
    use super::super::panel::{Panel, UniverseRule};
    use super::super::rules::SizingMode;
    use super::super::strategy::Policy;
    use super::*;
    use crate::market::Candle;

    const H: i64 = 3_600_000;

    /// One coin, hourly bars from (open, high, low, close) rows.
    fn panel(rows: &[(f64, f64, f64, f64)]) -> Panel {
        let c = rows.iter().enumerate().map(|(i, r)| Candle { open_time: i as i64 * H, open: r.0, high: r.1, low: r.2, close: r.3, volume: 1e6, close_time: (i as i64 + 1) * H - 1 }).collect();
        Panel::from_candles("t", "1h", vec![("X".into(), c)], Vec::new(), NO_RULE).unwrap()
    }

    fn plan_for(p: &Panel, entries: &[(usize, i8)], policy: Policy) -> Plan {
        let mut entry = vec![vec![0i8; p.n]; p.coins.len()];
        for (i, s) in entries {
            entry[0][*i] = *s;
        }
        Plan { entry, exit: Vec::new(), policy }
    }

    /// $1,000 per trade, no spread, slippage or funding, 10 bps fee: easy arithmetic.
    fn cfg() -> SimCfg {
        let costs = Costs { fee_bps: 10.0, slippage_bps: 0.0, spread_bps: 0.0, stop_slippage_bps: 0.0, ..Costs::default() };
        SimCfg::new(10_000.0, costs, Sizing { mode: SizingMode::Fixed, pos_pct: 0.10, ..Sizing::default() })
    }

    const HOLD3: Policy = Policy { hold_bars: 3, stop_bps: 0.0, extend: false, max_positions: 0 };

    #[test]
    fn fills_at_the_next_open_and_exits_on_time() {
        // Signal on bar 1 → buy at bar 2's open (100) → time stop 3 bars later at bar 5's open (110).
        let p = panel(&[(100.0, 101.0, 99.0, 100.0), (100.0, 101.0, 99.0, 100.0), (100.0, 104.0, 99.0, 103.0), (103.0, 107.0, 102.0, 106.0), (106.0, 109.0, 105.0, 108.0), (110.0, 111.0, 109.0, 110.0), (110.0, 111.0, 109.0, 110.0)]);
        let run = simulate(&p, &plan_for(&p, &[(1, 1)], HOLD3), &cfg());
        assert_eq!(run.trades.len(), 1);
        let t = &run.trades[0];
        assert_eq!((t.entry_i, t.exit_i, t.reason), (2, 5, Reason::Time));
        assert_eq!((t.entry_px, t.exit_px), (100.0, 110.0));
        // 10 coins: +$100, less $1.00 entry fee and $1.10 exit fee.
        assert!((t.pnl - 97.9).abs() < 1e-9, "{}", t.pnl);
        assert!((run.total_return() - 0.00979).abs() < 1e-12);
        assert!((run.fees - 2.1).abs() < 1e-9);
        assert_eq!(run.daily.len(), 1);
    }

    #[test]
    fn shorts_mirror_and_costs_are_paid_on_both_sides() {
        let p = panel(&[(100.0, 101.0, 99.0, 100.0), (100.0, 101.0, 99.0, 100.0), (100.0, 100.0, 95.0, 96.0), (96.0, 97.0, 94.0, 95.0), (95.0, 96.0, 94.0, 95.0), (90.0, 91.0, 89.0, 90.0)]);
        let mut c = cfg();
        c.costs.spread_bps = 4.0; // 2 bps a side
        c.costs.slippage_bps = 1.0; // + 1 bps a side
        let run = simulate(&p, &plan_for(&p, &[(1, -1)], HOLD3), &c);
        let t = &run.trades[0];
        assert_eq!(t.side, -1);
        assert!((t.entry_px - 100.0 * (1.0 - 3e-4)).abs() < 1e-9, "sold 3 bps under the open");
        assert!((t.exit_px - 90.0 * (1.0 + 3e-4)).abs() < 1e-9, "bought back 3 bps over it");
        let qty = 1000.0 / t.entry_px;
        let want = (t.entry_px - t.exit_px) * qty - 1.0 - qty * t.exit_px * 0.001;
        assert!((t.pnl - want).abs() < 1e-9);
        assert!(t.pnl > 95.0);
    }

    #[test]
    fn stops_fill_at_the_stop_or_at_a_gapped_open() {
        let pol = Policy { hold_bars: 10, stop_bps: 300.0, extend: false, max_positions: 0 };
        // Inside the bar: low 96 trades through the 97 stop → filled at 97.
        let p = panel(&[(100.0, 101.0, 99.0, 100.0), (100.0, 101.0, 99.0, 100.0), (100.0, 101.0, 99.5, 100.0), (100.0, 100.5, 96.0, 96.5), (96.0, 97.0, 95.0, 96.0)]);
        let run = simulate(&p, &plan_for(&p, &[(1, 1)], pol), &cfg());
        let t = &run.trades[0];
        assert_eq!((t.exit_i, t.reason), (3, Reason::Stop));
        assert!((t.exit_px - 97.0).abs() < 1e-9);
        // Gap: the bar opens at 94, under the stop → filled at 94, not 97.
        let p = panel(&[(100.0, 101.0, 99.0, 100.0), (100.0, 101.0, 99.0, 100.0), (100.0, 101.0, 99.5, 100.0), (94.0, 95.0, 93.0, 94.0), (94.0, 95.0, 93.0, 94.0)]);
        let run = simulate(&p, &plan_for(&p, &[(1, 1)], pol), &cfg());
        assert!((run.trades[0].exit_px - 94.0).abs() < 1e-9);
        // Stop slippage makes it worse still.
        let mut c = cfg();
        c.costs.stop_slippage_bps = 10.0;
        let run = simulate(&p, &plan_for(&p, &[(1, 1)], pol), &c);
        assert!((run.trades[0].exit_px - 94.0 * (1.0 - 1e-3)).abs() < 1e-9);
        // A stop can hit in the entry bar itself.
        let p = panel(&[(100.0, 101.0, 99.0, 100.0), (100.0, 101.0, 99.0, 100.0), (100.0, 100.0, 95.0, 96.0), (96.0, 97.0, 95.0, 96.0)]);
        let run = simulate(&p, &plan_for(&p, &[(1, 1)], pol), &cfg());
        assert_eq!((run.trades[0].entry_i, run.trades[0].exit_i), (2, 2));
    }

    #[test]
    fn funding_is_charged_to_longs_and_paid_to_shorts() {
        let rows = vec![(100.0, 100.0, 100.0, 100.0); 8];
        let mut p = panel(&rows);
        // +1 bps at each hour boundary: a long pays, a short receives.
        p.funding = vec![(0..8).map(|i| (i * H, 0.0001)).collect()];
        let mut c = cfg();
        c.costs.fee_bps = 0.0;
        let long = simulate(&p, &plan_for(&p, &[(1, 1)], HOLD3), &c);
        // Open over bars 2, 3, 4 → prints at the opens of 3, 4 and 5: 3 × $0.10.
        assert!((long.trades[0].funding + 0.30).abs() < 1e-9, "{}", long.trades[0].funding);
        assert!((long.trades[0].pnl + 0.30).abs() < 1e-9);
        let short = simulate(&p, &plan_for(&p, &[(1, -1)], HOLD3), &c);
        assert!((short.trades[0].pnl - 0.30).abs() < 1e-9);
        c.costs.funding = false;
        assert_eq!(simulate(&p, &plan_for(&p, &[(1, 1)], HOLD3), &c).trades[0].pnl, 0.0);
    }

    #[test]
    fn extend_keeps_a_position_instead_of_churning_it() {
        let rows = vec![(100.0, 100.0, 100.0, 100.0); 12];
        let p = panel(&rows);
        let sigs = [(1, 1), (4, 1), (7, -1)];
        let churn = simulate(&p, &plan_for(&p, &sigs, HOLD3), &cfg());
        assert_eq!(churn.trades.len(), 3, "long, long again, then short");
        let keep = simulate(&p, &plan_for(&p, &sigs, Policy { extend: true, ..HOLD3 }), &cfg());
        // One long from bar 2 to 8 (extended at 5), flipped to a short at bar 8.
        assert_eq!(keep.trades.len(), 2);
        assert_eq!((keep.trades[0].entry_i, keep.trades[0].exit_i, keep.trades[0].side), (2, 8, 1));
        assert_eq!((keep.trades[1].entry_i, keep.trades[1].side), (8, -1));
        assert!(keep.fees < churn.fees);
    }

    #[test]
    fn caps_and_delay() {
        let p = random_panel(3, 4, 6, "1h");
        let mut entry = vec![vec![0i8; p.n]; 4];
        for c in 0..4 {
            entry[c][30] = 1;
        }
        let pol = Policy { hold_bars: 5, stop_bps: 0.0, extend: false, max_positions: 2 };
        let plan = Plan { entry, exit: Vec::new(), policy: pol };
        let run = simulate(&p, &plan, &cfg());
        assert_eq!(run.trades.len(), 2, "two slots");
        // Gross cap: 4 × 10% wanted, 25% allowed → two full positions and a half.
        let mut c = cfg();
        c.sizing.max_gross = 0.25;
        let open = Plan { policy: Policy { max_positions: 0, ..pol }, ..plan.clone() };
        let run = simulate(&p, &open, &c);
        let mut sizes: Vec<f64> = run.trades.iter().map(|t| t.notional).collect();
        sizes.sort_by(f64::total_cmp);
        assert_eq!(sizes, vec![500.0, 1000.0, 1000.0]);
        // One bar of delay moves every entry one bar later.
        let mut c = cfg();
        c.entry_delay = 1;
        assert!(simulate(&p, &open, &c).trades.iter().all(|t| t.entry_i == 32));
    }

    #[test]
    fn inactive_coins_and_missing_bars_do_not_trade() {
        let rule = UniverseRule { min_dollar_volume: 1e12, min_history_days: 0 };
        let mut p = random_panel(3, 1, 6, "1h");
        p.rederive(rule);
        let run = simulate(&p, &plan_for(&p, &[(30, 1)], HOLD3), &cfg());
        assert!(run.trades.is_empty(), "nothing clears a $1T volume bar");
        let mut p = random_panel(3, 1, 6, "1h");
        p.open[0][31] = f64::NAN;
        let run = simulate(&p, &plan_for(&p, &[(30, 1)], HOLD3), &cfg());
        assert!(run.trades.is_empty(), "the fill bar is missing and the signal is not carried over");
    }

    #[test]
    fn daily_returns_add_up_to_trade_pnl() {
        let p = random_panel(9, 5, 30, "1h");
        let mut entry = vec![vec![0i8; p.n]; 5];
        let mut rng = crate::stats::Rng::new(1);
        for c in 0..5 {
            for i in 0..p.n {
                if rng.below(20) == 0 {
                    entry[c][i] = if rng.below(2) == 0 { 1 } else { -1 };
                }
            }
        }
        let plan = Plan { entry, exit: Vec::new(), policy: Policy { hold_bars: 7, stop_bps: 250.0, extend: false, max_positions: 0 } };
        let run = simulate(&p, &plan, &SimCfg::new(10_000.0, Costs::default(), Sizing::default()));
        assert!(run.trades.len() > 100);
        assert_eq!(run.daily.len(), 30);
        let from_trades: f64 = run.trades.iter().map(|t| t.pnl).sum();
        assert!((run.total_return() * 10_000.0 - from_trades).abs() < 1e-6);
        assert!((run.coin_pnl.iter().sum::<f64>() - from_trades).abs() < 1e-6);
        // No-edge random entries lose about the cost of trading.
        let cost = run.trades.iter().map(|t| t.notional * 13.0 / 1e4).sum::<f64>();
        assert!(from_trades < 0.0 || from_trades.abs() < 3.0 * cost);
        assert!(run.max_drawdown() > 0.0);
    }

    #[test]
    fn daily_loss_limit_blocks_new_entries_for_the_day() {
        // Bar 2 entry is stopped for −5%; the next signal the same day is skipped.
        let mut rows = vec![(100.0, 101.0, 99.0, 100.0); 30];
        rows[2] = (100.0, 100.0, 90.0, 91.0);
        let p = panel(&rows);
        let pol = Policy { hold_bars: 3, stop_bps: 500.0, extend: false, max_positions: 0 };
        let mut c = cfg();
        c.daily_loss_limit_pct = 0.004; // $40 of $10,000; the stop loses about $52
        let run = simulate(&p, &plan_for(&p, &[(1, 1), (6, 1), (25, 1)], pol), &c);
        assert_eq!(run.trades.len(), 2);
        assert_eq!(run.trades[1].entry_i, 26, "the next day trades again");
        c.daily_loss_limit_pct = 0.0;
        assert_eq!(simulate(&p, &plan_for(&p, &[(1, 1), (6, 1), (25, 1)], pol), &c).trades.len(), 3);
    }
}
