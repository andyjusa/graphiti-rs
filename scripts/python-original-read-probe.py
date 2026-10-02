#!/usr/bin/env python3
"""Private restored-data comparison. JSON stdin is never logged. No providers."""
import asyncio,ast,collections,hashlib,json,logging,os,sys,textwrap
from pathlib import Path
from types import SimpleNamespace
from typing import Any
assert os.environ.get('GRAPHITI_PRIVATE_READ_REHEARSAL')=='original-restore-read-only'
sys.path[:0]=['/tmp/read-source','/app/mcp/.venv/lib/python3.11/site-packages','/app/mcp/src']
import httpx
from graphiti_core.driver.falkordb_driver import FalkorDriver,FalkorGraph
from graphiti_core.nodes import EpisodicNode
from graphiti_core.edges import EntityEdge
from graphiti_core.errors import NodeNotFoundError
from graphiti_core.helpers import validate_group_id
from models.response_types import ErrorResponse,EpisodeSearchResponse
from utils.formatting import format_fact_result
from group_read_routing import *
from falkor_edge_query_compat import install
from pydantic import BaseModel
logging.disable(logging.CRITICAL)
request=json.load(sys.stdin)
assert request['scope']=='private original-memory read-only comparison'
readonly_queries=0
from falkordb.asyncio.graph import AsyncGraph as AsyncFalkorGraph
from redis.asyncio import Redis as AsyncRedis
redis_execute=AsyncRedis.execute_command
async def force_ro(self,*args,**kwargs):
 global readonly_queries
 command=args[0].decode() if isinstance(args[0],bytes) else str(args[0])
 command=command.upper()
 if command=='GRAPH.QUERY':args=('GRAPH.RO_QUERY',*args[1:]);command='GRAPH.RO_QUERY'
 assert command in {'GRAPH.RO_QUERY','GRAPH.LIST','PING','CLIENT','INFO','COMMAND'}, 'non-read Redis command refused'
 if command=='GRAPH.RO_QUERY':readonly_queries+=1
 return await redis_execute(self,*args,**kwargs)
AsyncRedis.execute_command=force_ro
install()
# Construct every driver before an event loop exists: no implicit index build.
groups=[g['group'] for g in request['groups'] if is_owner_group_authorized(g['group'])]
drivers={g:FalkorDriver(host='127.0.0.1',port=6379,database=g) for g in groups}
assert all(type(d._get_graph(g)) is AsyncFalkorGraph and isinstance(d.client.connection,AsyncRedis) for g,d in drivers.items())

class Service:
 current=None
 async def get_client(self):return SimpleNamespace(driver=drivers[self.current])
service=Service()
upstream=SimpleNamespace(graphiti_service=service,ErrorResponse=ErrorResponse,EpisodeSearchResponse=EpisodeSearchResponse,format_fact_result=format_fact_result,logger=logging.getLogger('private-sdk'),config=SimpleNamespace(graphiti=SimpleNamespace(group_id='user_primary')))
source=Path('/tmp/read-source/hermes_main.py').read_text();tree=ast.parse(source)
namespace=dict(globals())
for name in ['get_episode','_get_entity_edge_group_aware','_get_episodes_group_aware']:
 node=next(n for n in tree.body if isinstance(n,ast.AsyncFunctionDef) and n.name==name)
 segment=textwrap.dedent(''.join(source.splitlines(keepends=True)[node.lineno-1:node.end_lineno]))
 exec(compile(segment,'observed-hermes-read-wrapper','exec'),namespace)
def plain(value):return value.model_dump(mode='json') if isinstance(value,BaseModel) else value
class MCP:
 def __init__(self,url,token):
  self.url=url;self.id=0;self.session=None;self.version=None;self.headers={'Authorization':'Bearer '+token,'Host':'localhost:8000','Accept':'application/json, text/event-stream','Content-Type':'application/json'}
  self.client=httpx.AsyncClient(timeout=30,trust_env=False)
 async def rpc(self,method,params=None,notification=False):
  self.id+=1;body={'jsonrpc':'2.0','method':method}
  if not notification:body['id']=self.id
  if params is not None:body['params']=params
  h=dict(self.headers)
  if self.session:h['mcp-session-id']=self.session
  if self.version:h['mcp-protocol-version']=self.version
  response=await self.client.post(self.url,headers=h,json=body)
  if notification:
   assert response.status_code==202;return None
  assert response.status_code==200,('HTTP',response.status_code)
  if response.headers.get('mcp-session-id'):self.session=response.headers['mcp-session-id']
  value=response.json()
  assert 'error' not in value,('RPC error',value.get('error',{}).get('code'))
  return value['result']
 async def initialize(self):
  value=await self.rpc('initialize',{'protocolVersion':'2025-11-25','capabilities':{},'clientInfo':{'name':'private-restore-read-verifier','version':'1'}})
  assert value['protocolVersion']=='2025-11-25'
  self.version=value['protocolVersion']
  await self.rpc('notifications/initialized',notification=True)
  tools=await self.rpc('tools/list');golden=json.loads(Path('/tmp/read-source/production-mcp-contract.json').read_text())
  assert tools['tools']==golden['tools']['tools']
 async def tool(self,name,args):
  result=await self.rpc('tools/call',{'name':name,'arguments':args})
  assert result['isError'] is False
  value=result['structuredContent'] if name=='get_status' else result['structuredContent']['result']
  assert json.loads(result['content'][0]['text'])==value
  return value
 async def close(self):await self.client.aclose()
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
 direct=MCP('http://ingress:8000/mcp',request['backend_token'])
 oauth=MCP('http://graphiti-rs-read-oauth:8010/chatgpt/mcp',request['oauth_token'])
 await direct.initialize();await oauth.initialize()
 assert (await direct.tool('get_status',{}))['status']=='ok'
 assert (await oauth.tool('get_status',{}))['status']=='ok'
 summary=collections.Counter();path_counts=collections.Counter();missing_sample='00000000-0000-4000-8000-000000000001'
 for group in groups:
  service.current=group;driver=drivers[group]
  for kind,label,tool,wrapper in [('episode','Episodic','get_episode',namespace['get_episode']),('fact','RELATES_TO','get_entity_edge',namespace['_get_entity_edge_group_aware'])]:
   query='MATCH(n:Episodic) RETURN n.uuid AS uuid,n.group_id AS stored_group ORDER BY n.uuid' if kind=='episode' else 'MATCH()-[e:RELATES_TO]->() RETURN e.uuid AS uuid,e.group_id AS stored_group ORDER BY e.uuid'
   rows,_,_=await driver.execute_query(query)
   oauth_verified=False
   for row in rows:
    expected=plain(await wrapper(row['uuid'],group));actual=await direct.tool(tool,{'uuid':row['uuid'],'group_id':group})
    if row['stored_group']!=group:
     assert 'error' in expected and 'error' in actual
     summary[kind+'_scope_refusals']+=1;continue
    paths=differences(expected,actual);summary[kind+'_compared']+=1
    summary[kind+'_matched']+=int(not paths);path_counts.update(paths)
    if paths:
     print(json.dumps({'case':'point_read','kind':kind,'paths':paths}),flush=True)
    # Verify OAuth relay against a representative original response per group.
    if not oauth_verified:
     assert await oauth.tool(tool,{'uuid':row['uuid'],'group_id':group})==actual
     summary['oauth_original_reads']+=1;oauth_verified=True
   expected=plain(await wrapper(missing_sample,group));actual=await direct.tool(tool,{'uuid':missing_sample,'group_id':group})
   assert 'error' in expected or expected.get('found') is False
   assert ('error' in expected)==('error' in actual) and expected.get('found')==actual.get('found')
   summary[kind+'_missing_refusals']+=1
  expected=plain(await namespace['_get_episodes_group_aware']([group],10));actual=await direct.tool('get_episodes',{'group_ids':[group],'max_episodes':10})
  paths=differences(expected,actual);summary['episode_lists_compared']+=1;summary['episode_lists_matched']+=int(not paths);path_counts.update(paths)
  if paths:print(json.dumps({'case':'episode_list','paths':paths}),flush=True)
  print(json.dumps({'stage':'group_complete','groups_completed':sum(1 for g in groups[:groups.index(group)+1]),'totals':dict(summary)}),flush=True)
 assert readonly_queries>0
 await direct.close();await oauth.close()
 for driver in drivers.values():await driver.close()
 print(json.dumps({'stage':'complete','scope':'original-memory point/list read comparison only; search ranking and real embeddings not certified','policy_groups':len(groups),'outside_policy_groups':len(request['groups'])-len(groups),'totals':dict(summary),'mismatch_paths':dict(path_counts),'sdk_readonly_queries':readonly_queries,'provider_calls':0}),flush=True)
asyncio.run(main())
