#!/usr/bin/env python3
"""Owned Pi fixture: fulltext SDK/MCP differential and synthetic-only RDB restore.
Requires the exact labelled image/mount and a private, newly owned test root.
Never handles operational graphs/volumes, credentials or provider clients.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time
sys.path.insert(0, str(Path(__file__).resolve().parent))
import runpy
HELPERS=runpy.run_path(str(Path(__file__).with_name('pi-synthetic-validation.py')))
Redis,Server,uid=HELPERS['Redis'],HELPERS['Server'],HELPERS['uid']
CONTAINER='graphiti-rs-compat-pi-falkordb-1'
GROUP='project_pi_fulltext_20261002'
COUNT=1200
QUERIES=['cat','the cat','cats','CAT?','cat:missingterm','"cat"']

def inspected(root):
    info=json.loads(subprocess.check_output(['docker','inspect',CONTAINER]))[0]
    assert info['Config']['Labels']['task']=='graphiti-rs-compat-pi-synthetic'
    assert info['Config']['Labels']['com.docker.compose.project']=='graphiti-rs-compat-pi'
    assert info['Config']['Entrypoint']==['redis-server'] and not info['HostConfig']['PortBindings']
    assert info['Mounts'][0]['Source']==str(root/'db')
    assert info['Image']=='sha256:ed8f5d566429152635aaf6795ffd4e5bfd11f704ea03d6cc79fa25c41384d854'
    return info

def fingerprint(redis):
    records=redis.query(GROUP,'MATCH ()-[e:RELATES_TO]->() WITH e,startNode(e) AS n,endNode(e) AS m RETURN e.uuid,e.fact,n.uuid,m.uuid ORDER BY e.uuid')[1]
    return {'count':len(records),'sha256':hashlib.sha256(json.dumps(records,sort_keys=True,separators=(',',':')).encode()).hexdigest()}

def wait_for_indexes(redis):
    deadline=time.monotonic()+30
    states=[]
    while time.monotonic()<deadline:
        rows=redis.query(GROUP,"CALL db.indexes() YIELD label,status,types WHERE label IN ['Entity','RELATES_TO'] AND 'FULLTEXT' IN types['group_id'] RETURN label,status")[1]
        states.append(rows)
        if len(rows)==2 and all(row[1]=='OPERATIONAL' for row in rows):
            return states
        time.sleep(.05)
    raise RuntimeError('owned fulltext indexes did not become operational')

def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--test-root',required=True,type=Path)
    root=parser.parse_args().test_root.resolve()
    assert root.parent==Path('/home/pi4-3') and root.name.startswith('graphiti-rs-compat-pi.')
    assert root.stat().st_uid==os.getuid();inspected(root)
    report={'scope':'synthetic fulltext SDK/MCP and owned RDB restore, not operational restore/ingestion/cutover certification','actual_provider_calls':0,'status':'running'}
    redis=Redis();server=None
    try:
        assert redis.call('EXISTS',GROUP)==0
        stamp=json.dumps('2026-10-01T00:00:00.123456789Z')
        for first in range(0,COUNT,50):
            rows=[f'{{id:{json.dumps(uid(i+100000))},nid:{json.dumps(uid(i+200000))},name:{json.dumps("cat" if i==COUNT-1 else "catapult")}}}' for i in range(first,first+50)]
            redis.query(GROUP,f'CYPHER rows=[{",".join(rows)}] UNWIND $rows AS row MERGE(t:Entity:Company {{uuid:"{uid(999999)}"}}) SET t.name="Harbor",t.summary="Company",t.group_id="{GROUP}",t.created_at={stamp} CREATE(n:Entity:Person {{uuid:row.nid,name:row.name,summary:"Fixture",group_id:"{GROUP}",created_at:{stamp}}}) CREATE(n)-[e:RELATES_TO {{uuid:row.id,name:"VISITS",fact:row.name,group_id:"{GROUP}",created_at:{stamp},valid_at:{stamp},episodes:[]}}]->(t)')
        server=Server(root,'fulltext',{'COMPAT_DB_URL':'redis://127.0.0.1:16379'},compat=True)
        expected=uid(COUNT-1+100000);expected_node=uid(COUNT-1+200000)
        args={'query':'the cat','group_ids':[GROUP],'max_facts':10}
        before=server.tool('search_memory_facts',args)['facts']
        assert expected not in [f['uuid'] for f in before]
        redis.query(GROUP,"CALL db.idx.fulltext.createNodeIndex('Entity','name','summary','group_id')")
        redis.query(GROUP,'CREATE FULLTEXT INDEX FOR ()-[e:RELATES_TO]-() ON (e.name,e.fact,e.group_id)')
        report['index_readiness_before_search']=wait_for_indexes(redis)
        rust=[]
        for query in QUERIES:
            facts=server.tool('search_memory_facts',{'query':query,'group_ids':[GROUP],'max_facts':10})['facts']
            nodes=server.tool('search_nodes',{'query':query,'group_ids':[GROUP],'max_nodes':10,'entity_types':['Person']})['nodes']
            case={'query':query,'fact_ids':[f['uuid'] for f in facts],'node_ids':[n['uuid'] for n in nodes]}
            assert case['fact_ids']==[expected] and case['node_ids']==[expected_node],case
            rust.append(case)
        assert server.tool('search_memory_facts',{'query':'the and a','group_ids':[GROUP]})['facts']==[]
        assert server.tool('search_nodes',{'query':'cat','group_ids':[GROUP],'entity_types':['Company']})['nodes']==[]
        report['rust_cases']=rust
        report['substring_fallback_before_index']={'expected_absent':True,'returned_ids':[f['uuid'] for f in before]}
        inspected(root)
        for name in ('python-pi-fulltext.py','falkor_edge_query_compat.py'):
            subprocess.run(['docker','exec','-i','--user','0',CONTAINER,'sh','-c','cat > /tmp/'+name],input=(root/'source'/name).read_bytes(),check=True)
        result=subprocess.check_output(['docker','exec','--user','0','-w','/tmp',CONTAINER,'env','-i','PATH=/usr/local/bin:/usr/bin:/bin','GRAPHITI_PI_SYNTHETIC=graphiti-rs-compat-pi','SEMAPHORE_LIMIT=2','GRAPHITI_TELEMETRY_ENABLED=false','python3','-S','/tmp/python-pi-fulltext.py'])
        python=json.loads(result);assert python['cases']==rust
        report['python']=python;report['sdk_mcp_exact_id_lists_match']=True
        report['index_readiness_after_python']=wait_for_indexes(redis)
        original=fingerprint(redis);assert original['count']==COUNT
        report['fixture_before_restore']=original
        server.stop()
        # Preserve an owned synthetic RDB privately, then prove the active DB
        # is empty before cold-starting explicitly from the copied snapshot.
        assert redis.call('SAVE')=='OK'
        backup=root/'evidence/synthetic-backup.rdb';shutil.copy2(root/'db/dump.rdb',backup)
        report['rdb_snapshot']={'sha256':hashlib.sha256(backup.read_bytes()).hexdigest(),'bytes':backup.stat().st_size,'data_scope':'all graphs in this exclusively owned synthetic test DB; no operational data','published':False}
        assert redis.call('FLUSHALL')=='OK';assert redis.call('DBSIZE')==0
        redis.close();redis=None
        subprocess.run(['docker','stop',CONTAINER],check=True,stdout=subprocess.DEVNULL)
        shutil.copy2(backup,root/'db/dump.rdb')
        assert hashlib.sha256((root/'db/dump.rdb').read_bytes()).hexdigest()==report['rdb_snapshot']['sha256']
        overlay=root/'source/compose.restore.yaml'
        overlay.write_text('services:\n  falkordb:\n    command: ["--loadmodule", "/var/lib/falkordb/bin/falkordb.so", "--bind", "0.0.0.0", "--protected-mode", "no", "--dir", "/data", "--appendonly", "no", "--save", ""]\n')
        env={'PATH':'/usr/local/bin:/usr/bin:/bin','COMPAT_TEST_FALKOR_IMAGE':'sha256:ed8f5d566429152635aaf6795ffd4e5bfd11f704ea03d6cc79fa25c41384d854','COMPAT_TEST_UID':str(os.getuid()),'COMPAT_TEST_GID':str(os.getgid()),'COMPAT_TEST_ROOT':str(root)}
        command=['docker','compose','-f',str(root/'source/compose.compat-pi.yaml'),'-f',str(root/'source/compose.compat-pi-relay.yaml'),'-f',str(overlay)]
        subprocess.run(command+['up','-d','--force-recreate','--wait'],env=env,check=True,stdout=subprocess.DEVNULL)
        inspected(root);redis=Redis();restored=fingerprint(redis);assert restored==original
        report['index_readiness_after_restore']=wait_for_indexes(redis)
        server.start();after=server.tool('search_memory_facts',args)['facts'];assert [f['uuid'] for f in after]==[expected]
        report['synthetic_cold_restore']={'passed':True,'empty_active_db_confirmed_before_restore':True,'appendonly_disabled_for_restore':True,'fingerprint_after':restored,'indexed_search_after_restore_ids':[f['uuid'] for f in after]}
        report['status']='passed'
    finally:
        if server:
            server.stop();server.log.close()
        if redis: redis.close()
        if report['status']=='running':report['status']='failed; inspect console'
        (root/'evidence/fulltext-restore.json').write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps(report,indent=2))

if __name__=='__main__':main()

