#!/usr/bin/env python3
"""Rank liquid Binance USDT pairs by how often a 10-minute move clears the scalper's cost hurdle.

Usage: python3 -I scripts/vol_scan.py .env.test   (the env file supplies the testnet key, read-only,
to show which coins the testnet account holds for short inventory). Writes vol_scan.json next to the script."""
import json, sys, time, urllib.request, concurrent.futures as cf, statistics as st, os, hmac, hashlib
D="https://data-api.binance.vision/api/v3"; T="https://testnet.binance.vision/api/v3"
get=lambda u: json.load(urllib.request.urlopen(u, timeout=20))
FEE_RT=2*10+2*1  # bps: 2 fees + 2 slippage (spread added per pair)
COST_MULT=2.0
STABLE={"USDC","FDUSD","TUSD","USDP","DAI","EUR","TRY","BRL","AEUR","USDE","XUSD","BFUSD","USD1","PAXG","WBTC","WBETH","BNSOL"}
t24=[x for x in get(f"{D}/ticker/24hr") if x["symbol"].endswith("USDT")]
book={b["symbol"]:b for b in get(f"{D}/ticker/bookTicker")}
cands=[]
for x in t24:
    base=x["symbol"][:-4]
    if base in STABLE or base.endswith(("UP","DOWN","BULL","BEAR")): continue
    qv=float(x["quoteVolume"])
    if qv<20e6: continue
    b=book.get(x["symbol"]); 
    if not b or float(b["bidPrice"])<=0: continue
    spread=(float(b["askPrice"])/float(b["bidPrice"])-1)*1e4
    cands.append((x["symbol"],base,qv,spread))
cands.sort(key=lambda c:-c[2]); cands=cands[:60]
now=int(time.time()*1000)
def scan(c):
    sym,base,qv,spread=c
    rows=[]; t=now-2000*60000
    while t<now:
        r=get(f"{D}/klines?symbol={sym}&interval=1m&startTime={t}&limit=1000")
        if not r: break
        rows+=r; t=r[-1][0]+60000
    cl=[float(k[4]) for k in rows]
    if len(cl)<200: return None
    hurdle=COST_MULT*(FEE_RT+spread)
    m10=[abs(cl[i+10]/cl[i]-1)*1e4 for i in range(len(cl)-10)]
    r1=[(cl[i+1]/cl[i]-1)*1e4 for i in range(len(cl)-1)]
    sd1=st.pstdev(r1)
    return dict(sym=sym,base=base,qv_m=qv/1e6,spread=spread,hurdle=hurdle,med10=st.median(m10),
                p_over=sum(m>=hurdle for m in m10)/len(m10)*100, exp10=sd1*10**0.5, n=len(cl))
with cf.ThreadPoolExecutor(8) as ex: res=[r for r in ex.map(scan,cands) if r]
# testnet listing + balances
tn={s["symbol"] for s in get(f"{T}/exchangeInfo?permissions=SPOT")["symbols"] if s["status"]=="TRADING"}
bal={}
env={}
for line in open(sys.argv[1]):
    if "=" in line: k,v=line.strip().split("=",1); env[k]=v
q=f"timestamp={int(time.time()*1000)}&omitZeroBalances=true"
sig=hmac.new(env["BINANCE_SECRET_KEY"].encode(),q.encode(),hashlib.sha256).hexdigest()
req=urllib.request.Request(f"{T}/account?{q}&signature={sig}",headers={"X-MBX-APIKEY":env["BINANCE_API_KEY"]})
for b in json.load(urllib.request.urlopen(req,timeout=20))["balances"]: bal[b["asset"]]=float(b["free"])
res.sort(key=lambda r:-r["p_over"])
print(f"{'pair':<11}{'24h vol $M':>11}{'spread':>8}{'hurdle':>8}{'med 10m':>9}{'10m>hurdle':>12}{'σ·√10m':>8}  testnet  inventory")
for r in res[:30]:
    inv=bal.get(r["base"],0)
    print(f"{r['sym']:<11}{r['qv_m']:>11.0f}{r['spread']:>8.2f}{r['hurdle']:>8.1f}{r['med10']:>9.1f}{r['p_over']:>11.1f}%{r['exp10']:>8.1f}  {'yes' if r['sym'] in tn else 'NO ':<8} {inv:g} {r['base']}")
json.dump(res,open(os.path.join(os.path.dirname(os.path.abspath(sys.argv[0])),"vol_scan.json"),"w"),indent=1)
