"""Run on Pi host after comparison; restart only owned read-rehearsal services."""
import json,hashlib,subprocess,urllib.request,urllib.error,time
from pathlib import Path
root=Path(__file__).resolve().parent;m=json.loads((root/'resources.json').read_text())
names={'rust':'graphiti-rs-original-read-server','oauth':'graphiti-rs-read-oauth','bridge':'graphiti-rs-read-stateless'}
ids={k:m[k+'_container_id'] if k!='bridge' else m['stateless_container_id'] for k in names}
addresses={}
for component,name in names.items():
 info=json.loads(subprocess.check_output(['docker','inspect',name]))[0]
 assert info['Id']==ids[component] and info['Config']['Labels']['scope']=='private-original-read'
 addresses[component]=info['NetworkSettings']['Networks'][m['network']]['IPAddress']
backend_token=(root/'secrets/backend-token').read_text().strip();oauth_token=(root/'secrets/oauth-test-token').read_text().strip()
sequence=0;events=[]
def call(component,tool,args={}):
 global sequence
 sequence+=1
 version='2026-07-28' if component=='bridge' else '2025-11-25'
 params={'name':tool,'arguments':args}
 if component=='bridge':params['_meta']={'io.modelcontextprotocol/protocolVersion':version,'io.modelcontextprotocol/clientCapabilities':{}}
 body={'jsonrpc':'2.0','id':sequence,'method':'tools/call','params':params}
 headers={'Host':'localhost:8000','Authorization':'Bearer '+(oauth_token if component=='oauth' else backend_token),'Content-Type':'application/json','Accept':'application/json, text/event-stream','MCP-Protocol-Version':version,'Mcp-Method':'tools/call','Mcp-Name':tool}
 port,path=(8010,'/chatgpt/mcp') if component=='oauth' else (8001,'/mcp-stateless') if component=='bridge' else (8000,'/mcp')
 request=urllib.request.Request(f'http://{addresses[component]}:{port}{path}',headers=headers,data=json.dumps(body).encode())
 try:
  with urllib.request.urlopen(request,timeout=30) as response:
   status=response.status;raw=response.read().decode();content_type=response.headers.get('content-type','')
 except urllib.error.HTTPError as error:status=error.code;raw=error.read().decode();content_type=error.headers.get('content-type','')
 if content_type.startswith('text/event-stream'):raw=next(l[6:] for l in raw.splitlines() if l.startswith('data: '))
 try:payload=json.loads(raw)
 except json.JSONDecodeError:payload={}
 return status,payload
reference={}
for component in names:
 status,payload=call(component,'get_status');assert status==200 and payload['result']['structuredContent']['status']=='ok'
 status,payload=call(component,'get_episodes',{'group_ids':['user_primary'],'max_episodes':1});assert status==200
 value=payload['result']['structuredContent']['result'];reference[component]=hashlib.sha256(json.dumps(value,sort_keys=True).encode()).hexdigest()
assert len(set(reference.values()))==1
subprocess.run(['docker','stop','--time','10',ids['rust']],check=True,capture_output=True)
try:
 status,payload=call('bridge','get_status');assert payload.get('error',{}).get('code')==-32603
 events.append({'case':'bridge_backend_unavailable','http_status':status,'rpc_error_code':-32603,'write_or_retry_attempted_by_probe':False})
 status,_=call('oauth','get_status');assert status==502;events.append({'case':'oauth_backend_unavailable','http_status':status})
finally:
 subprocess.run(['docker','start',ids['rust']],check=True,capture_output=True)
 addresses['rust']=json.loads(subprocess.check_output(['docker','inspect',ids['rust']]))[0]['NetworkSettings']['Networks'][m['network']]['IPAddress']
for attempt in range(20):
 try:
  status,payload=call('rust','get_status');assert status==200 and payload['result']['structuredContent']['status']=='ok';break
 except (OSError,AssertionError):time.sleep(.25)
else:raise RuntimeError('owned read backend did not recover')
for component in names:
 status,payload=call(component,'get_status');assert status==200 and payload['result']['structuredContent']['status']=='ok'
 status,payload=call(component,'get_episodes',{'group_ids':['user_primary'],'max_episodes':1});assert status==200
 value=payload['result']['structuredContent']['result'];assert hashlib.sha256(json.dumps(value,sort_keys=True).encode()).hexdigest()==reference[component]
 events.append({'case':component+'_after_backend_restart','http_status':status,'original_episode_response_unchanged':True})
for component in ['oauth','bridge']:
 subprocess.run(['docker','restart','--time','10',ids[component]],check=True,capture_output=True)
 addresses[component]=json.loads(subprocess.check_output(['docker','inspect',ids[component]]))[0]['NetworkSettings']['Networks'][m['network']]['IPAddress']
 for attempt in range(20):
  try:
   status,payload=call(component,'get_status');assert status==200 and payload['result']['structuredContent']['status']=='ok';break
  except (OSError,AssertionError):time.sleep(.25)
 else:raise RuntimeError('owned read gateway did not recover')
 events.append({'case':component+'_gateway_restart','http_status':status,'seeded_test_token_still_accepted':True})
source=json.loads(subprocess.check_output(['docker','inspect',m['source_container_id']]))[0]
assert source['State']['StartedAt']==m['source_started'] and source['State']['Running'] and source['State']['Health']['Status']=='healthy'
report={'scope':'isolated read backend/gateway failure and recovery; seeded OAuth state only; operational switch/rollback and fresh GitHub login not exercised','events':events,'provider_calls':0,'original_service_unchanged_and_healthy':True}
(root/'evidence/reconnect-summary.json').write_text(json.dumps(report,indent=2));print(json.dumps(report))
