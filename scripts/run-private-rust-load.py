"""Run on Pi host after SDK comparison; only owned isolated read services."""
import hashlib,json,subprocess,time,threading,urllib.request,statistics
from pathlib import Path
root=Path(__file__).resolve().parent
resources=json.loads((root/'resources.json').read_text())
backend=json.loads(subprocess.check_output(['docker','inspect','graphiti-rs-original-read-server']))[0]
assert backend['Id']==resources['rust_container_id'] and backend['Config']['Labels']['scope']=='private-original-read'
address=backend['NetworkSettings']['Networks'][resources['network']]['IPAddress']
token=(root/'secrets/backend-token').read_text().strip()
headers={'Host':'localhost:8000','Authorization':'Bearer '+token,'Accept':'application/json, text/event-stream','Content-Type':'application/json','MCP-Protocol-Version':'2025-11-25'}
sequence=0
latencies=[];counts={};reference={}
def rpc(method,params=None,notification=False):
 global sequence
 sequence+=1;body={'jsonrpc':'2.0','method':method}
 if not notification:body['id']=sequence
 if params is not None:body['params']=params
 request=urllib.request.Request('http://'+address+':8000/mcp',headers=headers,data=json.dumps(body).encode(),method='POST')
 start=time.perf_counter()
 with urllib.request.urlopen(request,timeout=20) as response:
  status=response.status;data=response.read()
 elapsed=(time.perf_counter()-start)*1000
 if notification:assert status==202;return
 assert status==200
 value=json.loads(data);assert 'error' not in value
 return value['result'],elapsed
r,_=rpc('initialize',{'protocolVersion':'2025-11-25','capabilities':{},'clientInfo':{'name':'private-host-read-load','version':'1'}})
assert r['protocolVersion']=='2025-11-25'
rpc('notifications/initialized',notification=True)
queries={
 'get_episode':"MATCH(e:Episodic) WHERE e.group_id='user_primary' RETURN e.uuid ORDER BY e.uuid DESC LIMIT 1",
 'get_entity_edge':"MATCH()-[e:RELATES_TO]->() WITH e,startNode(e) AS s,endNode(e) AS t WHERE e.group_id='user_primary' AND s.group_id='user_primary' AND t.group_id='user_primary' RETURN e.uuid ORDER BY e.uuid DESC LIMIT 1",
}
uuids={}
for tool,query in queries.items():
 reply=json.loads(subprocess.check_output(['docker','exec',resources['db_container_id'],'redis-cli','--json','GRAPH.RO_QUERY','user_primary',query]))
 uuids[tool]=reply[1][0][0]
 assert isinstance(uuids[tool],str)
stop=threading.Event();samples=[];stats_errors=0
names={'graphiti-rs-original-read-server':'rust','graphiti-rs-original-restore-db':'database','graphiti-rs-read-oauth':'oauth','graphiti-rs-read-stateless':'stateless_bridge'}
def sample():
 global stats_errors
 while not stop.is_set():
  try:
   result=subprocess.run(['docker','stats','--no-stream','--format','{{json .}}',*names],capture_output=True,text=True,timeout=15)
   if result.returncode:stats_errors+=1
   for line in result.stdout.splitlines():
    value=json.loads(line);samples.append({'component':names[value['Name']],'cpu_percent':float(value['CPUPerc'].rstrip('%')),'memory_usage':value['MemUsage'].split(' / ')[0],'pids':int(value['PIDs'])})
  except Exception:stats_errors+=1
  stop.wait(1)
thread=threading.Thread(target=sample,daemon=True);thread.start()
started=time.perf_counter()
for i in range(192):
 tool=['get_episode','get_entity_edge'][i%2]
 result,elapsed=rpc('tools/call',{'name':tool,'arguments':{'uuid':uuids[tool],'group_id':'user_primary'}})
 value=result['structuredContent']['result'];assert 'error' not in value
 if tool=='get_episode':assert value['found']
 assert json.loads(result['content'][0]['text'])==value
 digest=hashlib.sha256(json.dumps(value,sort_keys=True,separators=(',',':')).encode()).hexdigest()
 if tool not in reference:reference[tool]=digest
 assert reference[tool]==digest
 latencies.append(elapsed);counts[tool]=counts.get(tool,0)+1
 time.sleep(0.1)
wall=time.perf_counter()-started;stop.set();thread.join(timeout=20)
ordered=sorted(latencies)
report={'scope':'bounded 192 sequential restored-memory point reads; host HTTP client; SDK comparison process absent; no provider calls or writes; not a production peak/load certification','tool_call_counts':counts,'responses_identical_across_repeats':True,'wall_seconds':round(wall,3),'latency_ms':{'min':round(min(ordered),3),'median':round(statistics.median(ordered),3),'p95':round(ordered[int(len(ordered)*.95)-1],3),'max':round(max(ordered),3)},'resource_samples':samples,'stats_errors':stats_errors,'provider_calls':0}
(root/'evidence/rust-read-load.json').write_text(json.dumps(report,indent=2))
print(json.dumps({k:v for k,v in report.items() if k!='resource_samples'}),flush=True)
