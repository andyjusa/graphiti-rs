#!/usr/bin/env python3
"""Observed provider methods, synthetic input only; no Graphiti/DB startup."""
import asyncio,ast,hashlib,importlib.metadata,json,os,runpy,sys,textwrap
from pathlib import Path
from types import SimpleNamespace
from typing import Any
assert os.environ.get('GRAPHITI_PROVIDER_REHEARSAL')=='synthetic-only'
sys.path.insert(0,'/app/mcp/.venv/lib/python3.11/site-packages')
from openai import AsyncOpenAI
import httpx
from graphiti_core.llm_client.client import LLMClient,get_extraction_language_instruction
from graphiti_core.prompts.extract_nodes import extract_text,ExtractedEntities
root=Path('/probe')
reference=json.loads((root/'rust-provider.json').read_text())
source=(root/'source/sitecustomize.py').read_text();tree=ast.parse(source)
cls=next(n for n in tree.body if isinstance(n,ast.ClassDef) and n.name=='ClusterGenericClient')
method=next(n for n in cls.body if isinstance(n,ast.AsyncFunctionDef) and n.name=='_generate_response')
excerpt=textwrap.dedent(''.join(source.splitlines(keepends=True)[method.lineno-1:method.end_lineno]))
helpers=runpy.run_path(str(root/'source/openrouter_json.py'))
namespace={'Any':Any,'json':json,'STRUCTURED_RESPONSE_MAX_ATTEMPTS':1,'STRUCTURED_RESPONSE_MAX_TOKENS':4096,'decode_first_json':helpers['decode_first_json'],'normalize_structured_response':helpers['normalize_structured_response']}
exec(compile(excerpt,'observed-patched-generation','exec'),namespace)
async def no_retry_sleep(delay):raise RuntimeError('provider_rehearsal_retry_disabled')
async def main():
 profile=json.loads((root/'profile.json').read_text())
 client=AsyncOpenAI(api_key=(root/'secrets/llm.key').read_text().strip(),base_url=profile['llm_base_without_credentials_or_query'],timeout=120,max_retries=0)
 wrapper=SimpleNamespace(client=client,model='gpt-6-luna',temperature=1,max_tokens=4096)
 wrapper._clean_input=lambda value:LLMClient._clean_input(wrapper,value)
 generated=[]
 for context in reference['contexts']:
  messages=extract_text(context);messages[0].content+=get_extraction_language_instruction()
  result=await namespace['_generate_response'](wrapper,messages,response_model=ExtractedEntities)
  generated.append(ExtractedEntities.model_validate(result).model_dump())
 print(json.dumps({"stage":"llm_complete","entities":generated},ensure_ascii=False),flush=True)
 await client.close()
 gemini=runpy.run_path(str(root/'source/gemini_embedding.py'))
 async with httpx.AsyncClient(timeout=120) as http:
  adapter=gemini['GeminiEmbedding2'](root/'secrets/gemini.key',client=http,sleep=no_retry_sleep)
  vectors=await adapter.create_batch(reference['texts'])
 return {'scope':'observed Python methods with bounded real requests and synthetic inputs; no DB access','openai_version':importlib.metadata.version('openai'),'method_source_sha256':hashlib.sha256(excerpt.encode()).hexdigest(),'generation_retries_disabled':True,'embedding_retries_disabled':True,'llm_calls':2,'embedding_batches':1,'entities':generated,'vectors':vectors}
try:print(json.dumps(asyncio.run(main()),ensure_ascii=False))
except Exception as error:
 import re
 code=str(error) if type(error).__name__=='GeminiEmbeddingError' and re.fullmatch('embedding_http_[0-9]{3}',str(error)) else None
 print(json.dumps({'status':'failed','error_class':type(error).__name__,'http_status':getattr(error,'status_code',None),'safe_error_code':code}));sys.exit(1)
