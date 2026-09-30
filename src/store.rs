use crate::domain::*;
use anyhow::{Context, Result, bail, ensure};
use chrono::DateTime;
use redis::{Value as RedisValue, aio::ConnectionManager};
use serde_json::{Value, json};
use std::time::Duration;

#[derive(Clone)]
pub struct Store { connection: ConnectionManager, graph: String }
impl Store {
    pub async fn connect(url: &str, graph: &str) -> Result<Self> {
        let client = redis::Client::open(url).context("invalid FalkorDB URL")?;
        let connection = tokio::time::timeout(Duration::from_secs(5),ConnectionManager::new(client)).await.context("FalkorDB connection timed out")?.context("FalkorDB connection failed")?;
        let s=Self{connection,graph:graph.into()};
        for query in ["CREATE INDEX FOR (e:Episode) ON (e.id)","CREATE INDEX FOR (n:Entity) ON (n.id)","CREATE INDEX FOR (m:MemoryGroup) ON (m.id)","CREATE INDEX FOR (n:Entity) ON (n.group_id)"] {
            s.query(query,json!({})).await?;
        }
        Ok(s)
    }
    pub async fn ping(&self) -> Result<()> { self.query("RETURN 1",json!({})).await?;Ok(()) }
    async fn query(&self,query:&str,params:Value)->Result<Vec<Vec<RedisValue>>> {
        let mut prefix=String::from("CYPHER ");
        for (name,value) in params.as_object().context("parameters must be an object")? {
            ensure!(name.chars().all(|c|c.is_ascii_alphanumeric()||c=='_'),"invalid parameter key");
            prefix.push_str(&format!("{name}={} ",cypher_value(value)?));
        }
        prefix.push_str(query);
        let mut connection=self.connection.clone();
        let result:RedisValue=redis::cmd("GRAPH.QUERY").arg(&self.graph).arg(prefix).arg("--compact").arg("TIMEOUT").arg(5000).query_async(&mut connection).await.context("FalkorDB query failed")?;
        let RedisValue::Array(top)=result else { bail!("unexpected FalkorDB response") };
        // Mutation-only replies contain statistics without a result table.
        let Some(RedisValue::Array(rows))=top.get(1) else{return Ok(vec![])};
        rows.iter().map(|row| match row { RedisValue::Array(cells)=>cells.iter().map(decode_cell).collect(), _=>bail!("invalid FalkorDB row") }).collect()
    }
    pub async fn episode(&self,group:&str,id:&str)->Result<Option<Episode>> {
        let rows=self.query("MATCH (e:Episode {id:$id}) RETURN e.data",json!({"id":stable_id(&[group,id])})).await?;
        rows.first().map(|r| serde_json::from_str(&redis_string(&r[0])?).map_err(Into::into)).transpose()
    }
    pub async fn ingest(&self,e:&Episode,x:&Extraction,vectors:Option<&[Vec<f32>]>,space:Option<&str>)->Result<IngestResult> {
        text(&e.group_id,128,"group_id")?;text(&e.id,128,"episode_id")?;text(&e.content,65536,"content")?;x.validate()?;
        if let Some(v)=vectors { ensure!(v.len()==x.facts.len(),"embedding count mismatch");ensure!(space.is_some(),"embedding space required"); for item in v { ensure!(!item.is_empty() && item.len()<=4096 && item.iter().all(|v|v.is_finite()),"invalid embedding"); } }
        let key=stable_id(&[&e.group_id,&e.id]);let token=uuid::Uuid::new_v4().to_string();
        let entities:Vec<Value>=x.entities.iter().map(|n| {
            let entity=Entity{id:stable_id(&[&e.group_id,&normalize(&n.name)]),group_id:e.group_id.clone(),name:n.name.trim().into(),summary:n.summary.clone()};
            json!({"id":entity.id,"name":normalize(&entity.name),"data":serde_json::to_string(&entity).unwrap()})
        }).collect();
        let mut facts=vec![];let mut superseded=vec![];let mut ids=vec![];
        for (i,f) in x.facts.iter().enumerate() {
            let source=stable_id(&[&e.group_id,&normalize(&f.source)]);let target=stable_id(&[&e.group_id,&normalize(&f.target)]);
            // Include the validity start so a fact can recur after an intervening invalidation.
            let id=stable_id(&[&e.group_id,&source,&target,&normalize(&f.relation),&normalize(&f.fact),&e.reference_time.to_rfc3339()]);
            ensure!(!f.supersedes.contains(&id),"fact cannot supersede itself");
            for old in &f.supersedes {
                let rows=self.query("MATCH ()-[r:FACT]->() WHERE r.group_id=$g AND r.id=$id RETURN r.valid_from, r.valid_to",json!({"g":e.group_id,"id":old})).await?;
                let row=rows.first().context("superseded fact does not exist in this group")?;
                let start=redis::from_redis_value::<i64>(&row[0])?;
                ensure!(start < e.reference_time.timestamp_millis(),"replacement must follow the old fact");
                if row[1]!=RedisValue::Nil { ensure!(redis::from_redis_value::<i64>(&row[1])? == e.reference_time.timestamp_millis(),"fact already superseded at a different time"); }
                superseded.push(old.clone());
            }
            let fact=Fact{id:id.clone(),group_id:e.group_id.clone(),source_id:source.clone(),target_id:target.clone(),source_name:f.source.clone(),target_name:f.target.clone(),relation:f.relation.clone(),fact:f.fact.clone(),valid_from:e.reference_time,valid_to:None,created_at:e.created_at,episodes:vec![],embedding:vectors.map(|v|v[i].clone())};
            facts.push(json!({"id":id,"source":source,"target":target,"text":normalize(&f.fact),"data":serde_json::to_string(&fact)?}));ids.push(id);
        }
        let params=json!({"g":e.group_id,"space":space.unwrap_or(""),"key":key,"token":token,"episode":serde_json::to_string(e)?,"content":e.content,"source":e.source,"time":e.reference_time.timestamp_millis(),"eid":e.id,"entities":entities,"facts":facts,"superseded":superseded,"ids":ids});
        let rows=self.query(r#"
            MERGE (m:MemoryGroup {id:$g})
            ON CREATE SET m.space=$space
            SET m.space=CASE WHEN m.space='' THEN $space ELSE m.space END
            WITH m WHERE $space='' OR m.space=$space
            MERGE (e:Episode {id:$key})
            ON CREATE SET e.data=$episode, e.group_id=$g, e.token=$token, e.content=$content, e.source=$source, e.time=$time, e.fact_ids=$ids
            WITH e WHERE e.content=$content AND e.source=$source AND e.time=$time
            WITH e, CASE WHEN e.token=$token THEN $entities ELSE [] END AS entities, CASE WHEN e.token=$token THEN $facts ELSE [] END AS facts
            FOREACH (n IN entities |
                MERGE (v:Entity {id:n.id}) ON CREATE SET v.group_id=$g, v.name=n.name, v.data=n.data)
            FOREACH (f IN facts |
                MERGE (a:Entity {id:f.source}) MERGE (b:Entity {id:f.target})
                MERGE (a)-[r:FACT {id:f.id}]->(b)
                ON CREATE SET r.group_id=$g, r.data=f.data, r.text=f.text, r.valid_from=$time, r.episodes=[]
                SET r.episodes=CASE WHEN $eid IN r.episodes THEN r.episodes ELSE r.episodes+[$eid] END)
            WITH e
            OPTIONAL MATCH ()-[old:FACT]->() WHERE old.group_id=$g AND old.id IN $superseded AND e.token=$token
            WITH e, collect(old) AS old_facts
            FOREACH (old IN old_facts | SET old.valid_to=$time)
            RETURN e.token=$token, e.fact_ids
        "#,params).await?;
        let row=rows.first().context("embedding space conflict or episode ID reused with different content/source/time")?;
        Ok(IngestResult{episode_id:e.id.clone(),inserted:redis::from_redis_value(&row[0])?,fact_ids:redis::from_redis_value(&row[1])?})
    }
    pub async fn delete_episode(&self,group:&str,id:&str)->Result<bool> {
        let rows=self.query(r#"
            MATCH (e:Episode {id:$key})
            OPTIONAL MATCH ()-[r:FACT]->() WHERE r.group_id=$g AND $eid IN r.episodes
            WITH e, collect(r) AS facts
            FOREACH (r IN facts | SET r.episodes=[x IN r.episodes WHERE x<>$eid])
            FOREACH (r IN [f IN facts WHERE size(f.episodes)=0] | DELETE r)
            DELETE e RETURN true
        "#,json!({"key":stable_id(&[group,id]),"g":group,"eid":id})).await?;
        Ok(!rows.is_empty())
    }
    pub async fn facts(&self,group:&str,at:i64,limit:usize)->Result<Vec<Fact>> { self.fact_candidates(group,at,limit,None).await }
    pub async fn fact_candidates(&self,group:&str,at:i64,limit:usize,keyword:Option<&str>)->Result<Vec<Fact>> {
        ensure!((1..=2000).contains(&limit),"candidate limit must be 1..=2000");
        let filter=if keyword.is_some(){"AND r.text CONTAINS $q"}else{""};
        let rows=self.query(&format!("MATCH ()-[r:FACT]->() WHERE r.group_id=$g AND r.valid_from<=$at AND (r.valid_to IS NULL OR $at<r.valid_to) {filter} RETURN r.data,r.valid_to,r.episodes ORDER BY r.valid_from DESC,r.id LIMIT $limit"),json!({"g":group,"at":at,"limit":limit,"q":keyword.unwrap_or("")})).await?;
        rows.into_iter().map(|row| {
            let mut fact:Fact=serde_json::from_str(&redis_string(&row[0])?)?;
            fact.valid_to=if row[1]==RedisValue::Nil {None}else{Some(DateTime::from_timestamp_millis(redis::from_redis_value(&row[1])?).context("invalid fact timestamp")?)};
            fact.episodes=redis::from_redis_value(&row[2])?; Ok(fact)
        }).collect()
    }
    pub async fn entities(&self,group:&str,query:&str,limit:usize)->Result<Vec<Entity>> {
        ensure!((1..=50).contains(&limit),"entity limit must be 1..=50");
        let rows=self.query("MATCH (n:Entity) WHERE n.group_id=$g AND n.name CONTAINS $q RETURN n.data ORDER BY n.name LIMIT $limit",json!({"g":group,"q":normalize(query),"limit":limit})).await?;
        rows.into_iter().map(|r|serde_json::from_str(&redis_string(&r[0])?).map_err(Into::into)).collect()
    }
    pub async fn embedding_space(&self,group:&str)->Result<Option<String>> {
        let rows=self.query("MATCH (m:MemoryGroup {id:$g}) RETURN m.space",json!({"g":group})).await?;
        rows.first().map(|r|redis_string(&r[0])).transpose()
    }
}
fn redis_string(v:&RedisValue)->Result<String>{redis::from_redis_value(v).map_err(Into::into)}
fn cypher_value(v:&Value)->Result<String>{
    Ok(match v {
        Value::Object(map)=>{let mut entries=vec![];for(k,v)in map {ensure!(k.chars().all(|c|c.is_ascii_alphanumeric()||c=='_'),"invalid map field");entries.push(format!("{k}:{}",cypher_value(v)?));}format!("{{{}}}",entries.join(","))},
        Value::Array(values)=>format!("[{}]",values.iter().map(cypher_value).collect::<Result<Vec<_>>>()?.join(",")),
        _=>serde_json::to_string(v)?,
    })
}

fn decode_cell(value:&RedisValue)->Result<RedisValue>{
    let RedisValue::Array(cell)=value else { bail!("expected compact FalkorDB value") };
    ensure!(cell.len()==2,"invalid compact cell");
    let kind:i64=redis::from_redis_value(&cell[0])?;
    Ok(match kind {
        1=>RedisValue::Nil,
        4=>RedisValue::Int(if redis_string(&cell[1])?=="true" {1}else{0}),
        6=>{let RedisValue::Array(values)=&cell[1] else {bail!("invalid compact array")};RedisValue::Array(values.iter().map(decode_cell).collect::<Result<Vec<_>>>()?)},
        2|3|5=>cell[1].clone(),
        _=>bail!("unsupported compact FalkorDB type {kind}"),
    })
}
