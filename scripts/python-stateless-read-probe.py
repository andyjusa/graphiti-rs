import asyncio,json,os,sys,hashlib
from pathlib import Path
assert os.environ.get('GRAPHITI_PRIVATE_READ_REHEARSAL')=='original-restore-read-only'
sys.path.insert(0,'/app/mcp/.venv/lib/python3.11/site-packages')
import httpx
request=json.load(sys.stdin)
assert request['scope']=='private stateless read comparison'
def differences(a,b,path=''):
 if type(a)!=type(b):return [path or '/']
 if isinstance(a,dict):
  paths=[]
  for k in sorted(set(a)|set(b)):
   p=path+'/'+k
   paths+= [p] if k not in a or k not in b else differences(a[k],b[k],p)
  return paths
 if isinstance(a,list):
  if len(a)!=len(b):return [path+'/length']
  return [p for i,(x,y) in enumerate(zip(a,b)) for p in differences(x,y,path+'/'+str(i))]
 return [] if a==b else [path or '/']
async def main():
 results=[];meta={'io.modelcontextprotocol/protocolVersion':'2026-07-28','io.modelcontextprotocol/clientInfo':{'name':'private-stateless-reader','version':'1'},'io.modelcontextprotocol/clientCapabilities':{},'io.modelcontextprotocol/logLevel':'info'}
 async with httpx.AsyncClient(timeout=30,trust_env=False) as c:
  for endpoint,url in [('direct','http://ingress:8000/mcp'),('bridge','http://graphiti-rs-read-stateless:8001/mcp-stateless')]:
   h={'Host':'localhost:8000','Authorization':'Bearer '+request['backend_token'],'Accept':'application/json, text/event-stream','Content-Type':'application/json','mcp-protocol-version':'2026-07-28'}
   async def rpc(method,params):
    params=dict(params,_meta=meta);h['mcp-method']=method
    if method=='tools/call':h['mcp-name']=params['name']
    else:h.pop('mcp-name',None)
    r=await c.post(url,headers=h,json={'jsonrpc':'2.0','id':1,'method':method,'params':params})
    assert r.status_code==200,(endpoint,method,r.status_code)
    if r.headers['content-type'].startswith('text/event-stream'):
     payload=next(line[6:] for line in r.text.splitlines() if line.startswith('data: '));value=json.loads(payload)
    else:value=r.json()
    assert 'error' not in value,(endpoint,method,value.get('error',{}).get('code'));return value['result']
   discovery=await rpc('server/discover',{});assert '2026-07-28' in discovery['supportedVersions']
   tools=(await rpc('tools/list',{}))['tools'];golden=json.loads(Path('/tmp/read-source/production-mcp-contract.json').read_text())['tools']['tools'];assert tools==golden
   status=await rpc('tools/call',{'name':'get_status','arguments':{}});assert status['structuredContent']['status']=='ok'
   lists=await rpc('tools/call',{'name':'get_episodes','arguments':{'group_ids':['user_primary'],'max_episodes':1}})
   if endpoint=='direct':original=lists
   else:
    print(json.dumps({'stage':'protocol_shape_diagnostic','mismatch_paths':differences(original,lists),'direct_fields':list(original),'bridge_fields':list(lists)}),flush=True)
    assert lists['structuredContent']==original['structuredContent']
    assert lists['content']==original['content'] and lists.get('isError',False)==original.get('isError',False)
   results.append({'endpoint':endpoint,'protocol_version':'2026-07-28','tools':len(tools),'schema_sha256':hashlib.sha256(json.dumps(tools,sort_keys=True,separators=(',',':')).encode()).hexdigest(),'original_episode_list_count':len(lists['structuredContent']['result']['episodes']),'source_mismatch':False})
 print(json.dumps({'scope':'native 2026 transport and observed stateless bridge with original read-only memory','results':results,'provider_calls':0}),flush=True)
asyncio.run(main())
