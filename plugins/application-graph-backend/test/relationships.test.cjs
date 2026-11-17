const {test}=require('node:test');
const assert=require('node:assert/strict');
const path=require('node:path');
const build=path.resolve(process.env.APPLICATION_GRAPH_TEST_BUILD||'local-dev/.generated/application-graph-tests');
const {relationships,structuralRelations}=require(path.join(build,'relationships.js'));
const {HttpError}=require(path.join(build,'catalog.js'));
function entity(ref,relations=[],annotations={}){
  const [kind,namespace,name]=ref.split(/[:/]/);
  return {kind:kind==='resource'?'Resource':'Component',metadata:{name,namespace,annotations},relations};
}
const c=name=>'component:default/'+name;
const r=name=>'resource:default/'+name;
test('standard component hierarchy excludes system membership',()=>{
  const parsed=structuralRelations(entity(c('child'),[{type:'partOf',targetRef:c('parent')},{type:'partOf',targetRef:'system:default/system'},{type:'dependsOn',targetRef:r('db')} ]));
  assert.deepEqual(parsed.relations,[{type:'partOf',targetRef:c('parent')},{type:'dependsOn',targetRef:r('db')}]);
  const raw={kind:'Component',metadata:{name:'child',namespace:'payments'},spec:{subcomponentOf:'parent',dependsOn:['component:payments/library','resource:payments/db']}};
  assert(structuralRelations(raw).relations.some(e=>e.type==='partOf'&&e.targetRef==='component:payments/parent'));
});
test('transitive catalog dependencies retain source evidence only for the used resource',async()=>{
  const entities=new Map([
    [c('app'),entity(c('app'),[{type:'dependsOn',targetRef:c('library')}])],
    [c('library'),entity(c('library'),[{type:'dependsOn',targetRef:r('used')},{type:'dependsOn',targetRef:r('unused')}])],
    [r('used'),entity(r('used'),[],{'application-graph.io/resource-uri':'file:used.txt'})],
    [r('unused'),entity(r('unused'),[],{'application-graph.io/resource-uri':'file:unused.txt'})],
  ]);
  const application={applicationKey:'key',entityRef:c('app'),graphVersion:'v1',dependencyTraces:[{coordinate:'g:library:1',paths:[{symbols:['a','b'],edges:[]}]}],
    resourceUses:[{uri:'file:used.txt',operation:'writeString',symbol:{nodeId:'b'},applicationPaths:[{symbols:['a','b'],edges:[]}]}]};
  const result=await relationships({request:{},readEntity:async(_req,ref)=>entities.get(ref),applications:[application,{applicationKey:'library',entityRef:c('library'),graphVersion:'v2',projectCoordinate:'g:library:1'}],entity:c('app'),depth:4});
  assert.equal(result.edges.find(e=>e.source===c('app')).status,'source-traced');
  assert.equal(result.dependencyPaths.find(p=>p.entities.at(-1)===r('used')).status,'source-traced');
  const unused=result.dependencyPaths.find(p=>p.entities.at(-1)===r('unused'));
  assert.equal(unused.status,'catalog-declared');assert.deepEqual(unused.codeEvidence,[]);
});
test('hasPart is structural and does not fabricate code calls',async()=>{
  const entities=new Map([[c('parent'),entity(c('parent'),[{type:'hasPart',targetRef:c('child')}])],[c('child'),entity(c('child'),[{type:'partOf',targetRef:c('parent')}])]]);
  const result=await relationships({request:{},readEntity:async(_req,ref)=>entities.get(ref),applications:[],entity:c('parent'),depth:4});
  assert.equal(result.nodes.length,2);assert.equal(result.edges.length,2);
  assert(result.edges.every(e=>e.status==='structural'&&e.codeEvidence.length===0));assert.deepEqual(result.dependencyPaths,[]);
});
test('cycles are bounded and inaccessible resources are omitted',async()=>{
  const entities=new Map([[c('a'),entity(c('a'),[{type:'dependsOn',targetRef:c('b')},{type:'dependsOn',targetRef:r('hidden')}])],[c('b'),entity(c('b'),[{type:'dependsOn',targetRef:c('a')}])]]);
  const result=await relationships({request:{},readEntity:async(_req,ref)=>{if(ref===r('hidden'))throw new HttpError(403,'DENIED','denied');return entities.get(ref);},applications:[],entity:c('a'),depth:8});
  assert.equal(result.nodes.length,2);assert(result.edges.every(e=>e.target!==r('hidden')));assert.equal(result.dependencyPaths.length,1);
});
test('catalog outages fail instead of looking like no dependencies',async()=>{
  await assert.rejects(relationships({request:{},readEntity:async()=>{throw new HttpError(502,'UNAVAILABLE','down');},applications:[],entity:c('app'),depth:4}),error=>error.status===502);
});
