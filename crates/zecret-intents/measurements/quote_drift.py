"""Asks 1Click for a dry quote every 20 s (20 USDC from NEAR to USDC on Base, SOL and
ETH on Base) and appends one JSON line per quote.

Usage: python3 -I quote_drift.py <out.jsonl> <seconds>
"""
import json,subprocess,datetime,time,sys
out=open(sys.argv[1],'a')
S="0d7550754e0800a5d237eef5826035766b9b3e5a15868a940ab289958788e3b0"
USDC="nep141:17208628f84f5d6ad33f0da3bbbeb27ffcb398eac501a31bd6ad2011e36133a1"
cases=[("usdc-base","nep141:base-0x833589fcd6edb6e08f4c7c32d4f71b54bda02913.omft.near","0x1111111111111111111111111111111111111111"),
       ("sol","nep141:sol.omft.near","13QkxhNMrTPxoCkRdYdJ65tFuwXPhL5gLS2Z5Nr6gjRK"),
       ("eth-base","nep141:base.omft.near","0x1111111111111111111111111111111111111111")]
end=time.time()+float(sys.argv[2])
while time.time()<end:
  for name,a,r in cases:
    dl=(datetime.datetime.now(datetime.UTC)+datetime.timedelta(minutes=30)).strftime('%Y-%m-%dT%H:%M:%S.000Z')
    q={"dry":True,"swapType":"EXACT_INPUT","slippageTolerance":100,"originAsset":USDC,"depositType":"INTENTS","destinationAsset":a,"amount":"20000000","refundTo":S,"refundType":"INTENTS","recipient":r,"recipientType":"DESTINATION_CHAIN","deadline":dl}
    t0=time.time()
    res=subprocess.run(["curl","-sS","-X","POST","https://1click.chaindefuser.com/v0/quote","-H","content-type: application/json","--data",json.dumps(q)],capture_output=True,text=True).stdout
    try:
      j=json.loads(res); qq=j.get('quote',{})
      rec={"t":datetime.datetime.now(datetime.UTC).isoformat(),"case":name,"ms":int((time.time()-t0)*1000),"amountOut":qq.get('amountOut'),"minAmountOut":qq.get('minAmountOut'),"timeEstimate":qq.get('timeEstimate'),"err":j.get('message')}
    except Exception as e:
      rec={"t":datetime.datetime.now(datetime.UTC).isoformat(),"case":name,"err":str(e)+res[:200]}
    out.write(json.dumps(rec)+"\n"); out.flush()
  time.sleep(20)
