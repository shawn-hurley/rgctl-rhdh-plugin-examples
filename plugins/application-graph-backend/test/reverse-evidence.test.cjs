const {test}=require('node:test');
const assert=require('node:assert/strict');
const path=require('node:path');
const build=path.resolve(process.env.APPLICATION_GRAPH_TEST_BUILD||'local-dev/.generated/application-graph-tests');
const {relationships}=require(path.join(build,'relationships.js'));
test('dependencyOf carries the same caller-owned evidence as dependsOn',async()=>{
  const app='component:default/app',resource='resource:default/files';
  const entities=new Map([
    [app,{kind:'Component',metadata:{name:'app'},relations:[{type:'dependsOn',targetRef:resource}]}],
    [resource,{kind:'Resource',metadata:{name:'files',annotations:{'application-graph.io/resource-uri':'file:orders.txt'}},relations:[{type:'dependencyOf',targetRef:app}]}],
  ]);
  const graph=await relationships({request:{},readEntity:async(_r,ref)=>entities.get(ref),entity:app,depth:3,
    applications:[{entityRef:app,applicationKey:'a',graphVersion:'v',resourceUses:[{uri:'file:orders.txt',operation:'writeString',symbol:{nodeId:'n'},applicationPaths:[{symbols:['n'],edges:[]}]}]}]});
  const direct=graph.edges.find(e=>e.type==='dependsOn'),reverse=graph.edges.find(e=>e.type==='dependencyOf');
  assert.equal(direct.status,'source-traced');assert.equal(reverse.status,'source-traced');
  assert.deepEqual(reverse.codeEvidence,direct.codeEvidence);assert.equal(reverse.codeEvidence[0].entityRef,app);
});
