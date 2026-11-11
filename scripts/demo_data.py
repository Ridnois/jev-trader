"""Write SYNTHETIC demo data for previewing the dashboard without a Jev key.

Usage: python3 scripts/demo_data.py /tmp/jev-demo/data
Then point a copy of config.toml's data_dir at that folder and run
`jev-trader --config <copy> dashboard`. Never point it at your real data dir.
Headlines are real titles; every Jev reading, price, trade and status is invented.
The folder gets a SYNTHETIC marker, so the dashboard labels it.
"""
import json, math, random, sys, os
from datetime import datetime, timedelta, timezone
random.seed(7)
if len(sys.argv) != 2: sys.exit(__doc__)
out = sys.argv[1]
if os.path.exists(os.path.join(out, "state.json")) and not os.path.exists(os.path.join(out, "SYNTHETIC")):
    sys.exit(f"{out} looks like real bot data (state.json without a SYNTHETIC marker); refusing to overwrite.")
os.makedirs(out, exist_ok=True)
now = datetime.now(timezone.utc).replace(microsecond=0)
iso = lambda d: d.isoformat().replace("+00:00","Z")
heads = [
 ("SOL","hack",-0.88,0.94,0.97,"Solana DEX drained of $40M in exploit","CoinDesk: Bitcoin, Ethereum, Crypto News and Price Data", 0.6),
 ("BTC","macro",0.36,0.88,0.91,"Trump Media moves $342 million in bitcoin to Crypto.com, blockchain data show","CoinDesk: Bitcoin, Ethereum, Crypto News and Price Data", 1.1),
 ("market","noise",0.05,0.97,0.12,"Here’s what happened in crypto today","Cointelegraph.com News",1.5),
 ("market","regulation",-0.22,0.71,0.64,"‘Lawyer up’: Democrats target Trump, Big Tech after midterm sweep","Cointelegraph.com News",1.8),
 ("market","hack",-0.41,0.66,0.78,"India warns AI-powered hackers targeting crypto exchanges at ‘machine speed’","Cointelegraph.com News",2.3),
 ("other","listing",0.12,0.55,0.70,"Mercado Bitcoin says phased reopening to begin Monday, will cover losses","Cointelegraph.com News",2.9),
 ("market","regulation",0.18,0.62,0.58,"Tokenization and stablecoins may weather Democrats' sweep as broader crypto rules face uncertainty","CoinDesk: Bitcoin, Ethereum, Crypto News and Price Data",3.2),
 ("market","noise",0.02,0.93,0.2,"Crypto's Washington hopes fade, but Grayscale and BlackRock see growth ahead","CoinDesk: Bitcoin, Ethereum, Crypto News and Price Data",3.3),
 ("BTC","partnership",0.28,0.74,0.81,"Why Bitcoin's Next Soft Fork Could Feel Like a Practice Run","Decrypt",3.35),
 ("other","hack",-0.5,0.9,0.85,"BitMart founder says security team ‘severely understaffed’ after new hack","Cointelegraph.com News",3.6),
 ("other","noise",0.0,0.95,0.1,"ChatGPT Is Getting a GIF Maker Powered by OpenAI's Unannounced Images 2.6 Model","Decrypt",3.9),
 ("market","noise",0.0,0.9,0.15,"How Aussie Bitcoiners can avoid 2027’s shock CGT hike","Cointelegraph.com News",4.6),
 ("market","hack",0.22,0.6,0.72,"Coinbase Says 99% of Customer Crypto Is Offline and Out of Hackers’ Reach","Decrypt",5.4),
 ("BTC","macro",0.55,0.86,0.9,"Spot bitcoin ETFs log $612M in daily inflows, the largest since August","CoinDesk: Bitcoin, Ethereum, Crypto News and Price Data",5.9),
 ("ETH","partnership",0.31,0.7,0.76,"Ethereum's Fusaka upgrade sets mainnet date after final testnet","Decrypt",6.4),
]
events=[]
for i,(a,ev,s,ec,ni,title,src,hago) in enumerate(heads):
    pub = now - timedelta(hours=hago)
    events.append(dict(id=f"https://example.invalid/synthetic/{i}", title=title, source=src, published=iso(pub), analyzed_at=iso(pub+timedelta(seconds=40)),
        asset=a, asset_conf=round(random.uniform(.7,.99),2), event=ev, event_conf=ec, sentiment=s, sentiment_conf=round(random.uniform(.45,.8),2), new_info=ni, model="jev-1.13.0"))
events.sort(key=lambda e:e["analyzed_at"])
with open(f"{out}/events.jsonl","w") as f:
    for e in events: f.write(json.dumps(e)+"\n")
ev_id = {e["title"][:20]:e["id"] for e in events}
def eid(prefix): return [v for k,v in ev_id.items() if k.startswith(prefix[:20])][0]

base = {"BTC":79200,"ETH":3240,"SOL":156}
ticks=[]; equity=[]; pipe=[]
N=360
eq=10000.0
for k in range(N):
    ts = now - timedelta(seconds=60*(N-k)) + timedelta(seconds=40)
    hrs_ago = (N-k)/60
    for sym in base:
        drift = {"BTC":0.00004,"ETH":-0.00001,"SOL":-0.00002}[sym]
        if sym=="SOL" and hrs_ago<0.7: drift=-0.0004
        base[sym]*= (1+drift+random.gauss(0,0.0008))
        if sym=="BTC": sc = 0.1 + (0.32 if hrs_ago<5.9 else 0) + (0.1 if hrs_ago<1.1 else 0) - 0.02*hrs_ago/6
        elif sym=="SOL": sc = 0.05 - (0.66 if hrs_ago<0.6 else 0)
        else: sc = 0.02 + (0.06 if hrs_ago<6.4 else 0)
        sc += random.gauss(0,0.01)
        w = 0.18 if (sym=="BTC" and hrs_ago<5.85) else (0.12 if (sym=="SOL" and 0.6<hrs_ago<4.0) else 0.0)
        reason = {"BTC":"news %+.2f, momentum %+.2f%%"%(sc,1.8), "ETH":"hold", "SOL":"hack alert" if hrs_ago<0.6 else "hold"}[sym]
        drivers = {"BTC":[eid("Spot bitcoin ETFs"),eid("Trump Media"),eid("Why Bitcoin's Next")],"SOL":[eid("Solana DEX")] if hrs_ago<0.6 else [],"ETH":[eid("Ethereum's Fusaka")]}[sym]
        ticks.append(dict(ts=iso(ts),asset=sym,price=round(base[sym],2),momentum=round({"BTC":0.018,"ETH":-0.004,"SOL":-0.026}[sym]+random.gauss(0,.001),4),
            news=dict(score=round(sc,3),n_events=3,hack_alert=(sym=="SOL" and hrs_ago<0.6)),target={"Weight":w} if w else "Hold",final_weight=w,halted=False,reason=reason,drivers=drivers))
    eq += random.gauss(0.3,2.2)
    equity.append(dict(ts=iso(ts),equity=round(eq,2),cash=8200.0))
    jstate = "degraded" if k in (N-4, N-37, N-120) else ("skipped" if k%3 else "ok")
    stages=[dict(name="Feeds",state="ok" if k!=N-200 else "degraded",detail="3/3 sources · 41 items · %d new"%(0 if k%3 else 2),ms=random.randint(300,900)),
      dict(name="Jev",state=jstate,detail={"ok":"2 analyzed · p50 %dms"%random.randint(140,260),"skipped":"no new headlines","degraded":"1/2 failed, retrying next tick · Jev request failed: operation timed out"}[jstate],ms=random.randint(0,3100) if jstate!="skipped" else 0),
      dict(name="Prices",state="ok",detail="3 pairs · 1h candles",ms=random.randint(180,420)),
      dict(name="Signals",state="ok",detail="BTC %+.2f · ETH +0.08 · SOL %+.2f"%(0.42,-0.61 if hrs_ago<0.6 else 0.05),ms=0),
      dict(name="Risk",state="ok",detail="gross 18%%/60%% · day %+.1f%%/−3%%"%(-0.4),ms=0),
      dict(name="Execution",state="ok",detail="no orders" if k!=N-36 else "1 paper fills",ms=1)]
    pipe.append(dict(ts=iso(ts),tick=4112-N+k+1,stages=stages))
for name,rows in (("ticks",ticks),("equity",equity),("pipeline",pipe)):
    with open(f"{out}/{name}.jsonl","w") as f:
        for r in rows: f.write(json.dumps(r)+"\n")
trades=[
 dict(ts=iso(now-timedelta(hours=5.85)),asset="BTC",side="buy",qty=0.022801,price=79610.4,notional=1815.2,fee=1.82,reason="news +0.43, momentum +1.62%",events=[eid("Spot bitcoin ETFs"),eid("Why Bitcoin's Next")]),
 dict(ts=iso(now-timedelta(hours=4.0)),asset="SOL",side="buy",qty=7.74,price=155.1,notional=1200.47,fee=1.2,reason="news +0.34, momentum +0.80%",events=[]),
 dict(ts=iso(now-timedelta(minutes=34)),asset="SOL",side="sell",qty=7.74,price=148.15,notional=1146.68,fee=1.15,reason="hack alert",events=[eid("Solana DEX")]),
]
with open(f"{out}/trades.jsonl","w") as f:
    for r in trades: f.write(json.dumps(r)+"\n")
last = now - timedelta(seconds=20)
status=dict(pid=1,started_at=iso(now-timedelta(hours=6,minutes=3)),last_tick_at=iso(last),next_tick_at=iso(last+timedelta(seconds=60)),tick=4112,interval_secs=60,paused=False,halted=False,stages=pipe[-1]["stages"])
json.dump(status,open(f"{out}/status.json","w"),indent=1)
state=dict(cash=8200.0,positions={"BTC":{"qty":0.022801,"entry":79610.4}},day=now.date().isoformat(),day_start_equity=10225.0,halted_on=None,cooldown_until={"SOL":iso(now+timedelta(hours=5,minutes=26))},seen=[],events=[],flatten_handled=None)
json.dump(state,open(f"{out}/state.json","w"),indent=1)
json.dump(dict(paused=False,flatten_requested_at=None),open(f"{out}/control.json","w"))
# A synthetic backtest run (data/backtests/<run_id>/), shaped like `jev-trader backtest` output.
def demo_backtest():
    rnd = random.Random(11)
    hours = 24*21
    t0 = (now - timedelta(hours=hours+1)).replace(minute=0, second=0)
    px0 = {"BTC":78000.0,"ETH":3100.0,"SOL":150.0}
    paths = {}
    for sym, p0 in px0.items():
        p, row = p0, []
        for h in range(hours):
            p *= 1 + rnd.gauss(0.00005, 0.006 if sym!="BTC" else 0.004)
            row.append(p)
        paths[sym] = row
    def sim(name, entry_p, seed):
        r = random.Random(seed)
        cash, pos, eq, fills = 10000.0, {}, [], []
        for h in range(hours):
            ts = t0 + timedelta(hours=h+1)
            for sym in px0:
                price = paths[sym][h]
                if sym in pos and (r.random() < 0.04 or price < pos[sym][1]*0.96):
                    qty, entry, cost = pos.pop(sym)
                    notional = qty*price; fee = notional*0.001
                    cash += notional - fee
                    pnl = notional - fee - cost
                    reason = "stop loss" if price < entry*0.96 else r.choice(["news turned negative","news faded below exit","momentum turned down"])
                    fills.append(dict(ts=iso(ts),asset=sym,side="sell",qty=round(qty,6),price=round(price,2),notional=round(notional,2),fee=round(fee,2),reason=reason,events=[],realized_pnl=round(pnl,2),round_trip_pnl=round(pnl,2)))
                elif sym not in pos and len(pos) < 2 and r.random() < entry_p:
                    notional = min(cash*0.2, 2000.0); fee = notional*0.001
                    qty = notional/price; cash -= notional + fee
                    pos[sym] = (qty, price, notional+fee)
                    fills.append(dict(ts=iso(ts),asset=sym,side="buy",qty=round(qty,6),price=round(price,2),notional=round(notional,2),fee=round(fee,2),reason="news %+.2f, momentum %+.2f%%"%(r.uniform(0.3,0.7),r.uniform(0.2,2.2)),events=[]))
            eq.append((iso(ts), round(cash + sum(q*paths[s][h] for s,(q,_,_) in pos.items()), 2)))
        return eq, fills
    run_id = "demo_%s_%s" % (t0.strftime("%Y%m%d"), (t0+timedelta(hours=hours)).strftime("%Y%m%d"))
    d = os.path.join(out, "backtests", run_id); os.makedirs(d, exist_ok=True)
    for name, ep, seed in (("strategy",0.03,1),("momentum_only",0.05,2),("news_only",0.02,3)):
        eq, fills = sim(name, ep, seed)
        with open(f"{d}/equity_{name}.jsonl","w") as f:
            for e in eq: f.write(json.dumps(e)+"\n")
        with open(f"{d}/trades_{name}.jsonl","w") as f:
            for x in fills: f.write(json.dumps(x)+"\n")
        if name == "strategy": s_eq, s_fills = eq, fills
    peak, mdd = 10000.0, 0.0
    for _, v in s_eq: peak = max(peak, v); mdd = max(mdd, 1 - v/peak)
    summary = dict(label="backtest · replay of cached events", synthetic=True, run_id=run_id, **{"from": iso(t0), "to": iso(t0+timedelta(hours=hours))},
        verdict="BASELINE_EXPLAINED",
        strategy=dict(total_return=s_eq[-1][1]/10000-1, max_drawdown=mdd, hours=hours, fills=len(s_fills), turnover=sum(x["notional"] for x in s_fills)/10000, fees=sum(x["fee"] for x in s_fills), decision_clusters=31, decision_hit_rate=0.52),
        primary=dict(baseline="momentum-only (primary, declared)", ols=dict(alpha=0.00002, t_alpha=0.8, beta=0.71, r2=0.44, resid_sd=0.002), edge_ratio=0.01, stability=0.5),
        alternate=dict(baseline="BTC buy-and-hold (alternate)", ols=dict(alpha=0.00003, t_alpha=1.1, beta=0.22, r2=0.09, resid_sd=0.003), edge_ratio=0.01, stability=0.5))
    json.dump(summary, open(f"{d}/summary.json","w"), indent=1)
demo_backtest()
open(f"{out}/SYNTHETIC","w").write("demo data written by scripts/demo_data.py\n")
print("wrote synthetic demo data to", out)
