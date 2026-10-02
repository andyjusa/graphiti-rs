import asyncio,json,os,sys,hashlib
assert os.environ.get('GRAPHITI_PRIVATE_READ_REHEARSAL')=='original-restore-read-only'
sys.path.insert(0,'/app/mcp/.venv/lib/python3.11/site-packages')
import httpx
request=json.load(sys.stdin)
assert request['scope']=='private original-memory auth probe'
async def main():
 results=[]
 async with httpx.AsyncClient(timeout=20,trust_env=False) as c:
  direct='http://ingress:8000/mcp';oauth='http://graphiti-rs-read-oauth:8010/chatgpt/mcp'
  init={'jsonrpc':'2.0','id':1,'method':'initialize','params':{'protocolVersion':'2025-11-25','capabilities':{},'clientInfo':{'name':'private-auth-probe','version':'1'}}}
  base={'Host':'localhost:8000','Accept':'application/json, text/event-stream','Content-Type':'application/json'}
  for name,url,token,origin,host in [('backend_missing',direct,None,None,None),('backend_wrong',direct,'wrong',None,None),('backend_oauth_token',direct,request['oauth_token'],None,None),('backend_origin',direct,request['backend_token'],'https://example.invalid',None),('backend_host',direct,request['backend_token'],None,'example.invalid'),('oauth_missing',oauth,None,None,None),('oauth_wrong',oauth,'wrong',None,None),('oauth_backend_token',oauth,request['backend_token'],None,None),('oauth_origin',oauth,request['oauth_token'],'https://example.invalid',None)]:
   h=dict(base)
   if token:h['Authorization']='Bearer '+token
   if origin:h['Origin']=origin
   if host:h['Host']=host
   r=await c.post(url,headers=h,json=init)
   assert 400<=r.status_code<500,(name,r.status_code)
   if name.startswith('oauth_'):assert r.status_code==401 and 'resource_metadata=' in r.headers.get('www-authenticate','')
   results.append({'case':name,'status':r.status_code,'oauth_metadata_challenge': 'resource_metadata=' in r.headers.get('www-authenticate','')})
  for name,url,token in [('backend',direct,request['backend_token']),('oauth',oauth,request['oauth_token'])]:
   h=dict(base,Authorization='Bearer '+token)
   r=await c.post(url,headers=h,json=init);assert r.status_code==200
   version=r.json()['result']['protocolVersion'];assert version=='2025-11-25'
   results.append({'case':name+'_initialize','status':200,'protocol_version':version,'session_header_present':'mcp-session-id' in r.headers})
   h['mcp-protocol-version']=version
   r=await c.post(url,headers=h,json={'jsonrpc':'2.0','method':'notifications/initialized'});assert r.status_code==202
   r=await c.post(url,headers=h,json={'jsonrpc':'2.0','id':2,'method':'tools/list'});assert r.status_code==200
   tools=r.json()['result']['tools'];assert len(tools)==10
   results.append({'case':name+'_tools','tools':len(tools),'schema_sha256':hashlib.sha256(json.dumps(tools,sort_keys=True,separators=(',',':')).encode()).hexdigest()})
   body={'jsonrpc':'2.0','id':3,'method':'tools/call','params':{'name':'get_status','arguments':{}}}
   r=await c.post(url,headers=h,json=body);assert r.status_code==200 and r.json()['result']['structuredContent']['status']=='ok'
   h.pop('mcp-protocol-version');r=await c.post(url,headers=h,json=body);assert r.status_code==200 and r.json()['result']['structuredContent']['status']=='ok'
   results.append({'case':name+'_legacy_default_protocol_header','status':r.status_code})
  r=await c.get('http://graphiti-rs-read-oauth:8010/.well-known/oauth-protected-resource/chatgpt/mcp');assert r.status_code==200
  d=r.json();assert d['resource'].endswith('/chatgpt/mcp') and len(d['authorization_servers'])==1
  results.append({'case':'oauth_discovery','status':200,'resource_suffix_verified':True})
 print(json.dumps({'scope':'isolated original-memory read services; seeded test OAuth tokens only; fresh GitHub login not tested','cases':results,'provider_calls':0}),flush=True)
asyncio.run(main())
