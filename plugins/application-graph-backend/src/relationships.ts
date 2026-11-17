import type {Request} from 'express';
import {Entity,HttpError,entityRef} from './catalog';

type Kind = 'component' | 'resource';
export function catalogRef(value: unknown, defaultKind: Kind = 'component', namespace = 'default') {
  const kind = typeof value === 'string' && value.includes(':') ? value.split(':')[0].toLowerCase() : defaultKind;
  if (kind!=='component' && kind!=='resource') throw new HttpError(400,'INVALID_ENTITY','Expected a Component or Resource');
  return entityRef(value,kind,namespace);
}
export function structuralRelations(entity: Entity) {
  const namespace=entity.metadata.namespace??'default';
  const ref=catalogRef(`${entity.kind}:${namespace}/${entity.metadata.name}`);
  const sourceKind=entity.kind.toLowerCase();
  const relations:{type:string;targetRef:string}[]=[];
  if(entity.relations!==undefined) {
    for(const relation of entity.relations) {
      if(!['dependsOn','dependencyOf','partOf','hasPart'].includes(relation.type))continue;
      const targetKind=relation.targetRef.split(':')[0].toLowerCase();
      if(!['component','resource'].includes(targetKind))continue;
      if(['partOf','hasPart'].includes(relation.type) && (sourceKind!=='component'||targetKind!=='component'))continue;
      relations.push({type:relation.type,targetRef:catalogRef(relation.targetRef,'component',namespace)});
    }
  } else {
    for(const value of entity.spec?.dependsOn??[])relations.push({type:'dependsOn',targetRef:catalogRef(value,'resource',namespace)});
    for(const value of entity.spec?.dependencyOf??[])relations.push({type:'dependencyOf',targetRef:catalogRef(value,'resource',namespace)});
    if(sourceKind==='component'&&entity.spec?.subcomponentOf)relations.push({type:'partOf',targetRef:entityRef(entity.spec.subcomponentOf,'component',namespace)});
  }
  return {ref,relations};
}
interface Trace {symbols:unknown[];edges:unknown[]}
export interface GraphEvidence {
  applicationKey:string;entityRef:string;graphVersion:string;projectCoordinate?:string;
  dependencyTraces?:{coordinate:string;paths:Trace[]}[];
  resourceUses?:{uri?:string;operation:string;symbol:unknown;applicationPaths:Trace[]}[];
}
export async function relationships(options:{request:Request;readEntity:(request:Request,ref:string)=>Promise<Entity>;
  applications:GraphEvidence[];entity?:string;depth:number}) {
  const indexed=new Map(options.applications.map(a=>[a.entityRef,a]));
  const seeds=options.entity?[options.entity]:[...indexed.keys()];
  const loaded=new Map<string,Entity>();
  const visited=new Set<string>();
  const candidates:{source:string;target:string;type:string}[]=[];
  const queue=seeds.map(ref=>({ref,depth:0}));
  let truncated=false;
  while(queue.length) {
    const next=queue.shift()!;
    if(visited.has(next.ref))continue;
    visited.add(next.ref);
    if(loaded.size>=200){truncated=true;break;}
    let entity:Entity;
    try { entity=await options.readEntity(options.request,next.ref); }
    catch(error){
      if(next.ref===options.entity)throw error;
      if(error instanceof HttpError&&[403,404].includes(error.status))continue;
      throw error;
    }
    const parsed=structuralRelations(entity);
    if(parsed.ref!==next.ref)throw new HttpError(502,'CATALOG_IDENTITY_MISMATCH','Catalog returned a different entity');
    loaded.set(next.ref,entity);
    for(const relation of parsed.relations) {
      candidates.push({source:next.ref,target:relation.targetRef,type:relation.type});
      if(next.depth<options.depth)queue.push({ref:relation.targetRef,depth:next.depth+1});
    }
  }
  const evidence=(from:string,to:string)=>{
    const graph=indexed.get(from); const target=loaded.get(to); const other=indexed.get(to);
    if(!graph||!target)return [];
    const result:unknown[]=[];
    if(other?.projectCoordinate) {
      for(const dependency of graph.dependencyTraces??[])if(dependency.coordinate===other.projectCoordinate&&dependency.paths.length)
        result.push({kind:'maven-calls',coordinate:dependency.coordinate,entityRef:from,graphVersion:graph.graphVersion,paths:dependency.paths});
    }
    const uri=target.metadata.annotations?.['application-graph.io/resource-uri'];
    if(target.kind.toLowerCase()==='resource'&&uri) {
      for(const use of graph.resourceUses??[])if(use.uri===uri&&use.applicationPaths.length)
        result.push({kind:'resource-use',uri,operation:use.operation,entityRef:from,graphVersion:graph.graphVersion,symbol:use.symbol,paths:use.applicationPaths});
    }
    return result;
  };
  const unique=new Map(candidates.filter(e=>loaded.has(e.source)&&loaded.has(e.target)).map(e=>[`${e.source}|${e.type}|${e.target}`,e]));
  const edges=[...unique.values()].map(edge=>{
    const codeEvidence=edge.type==='dependsOn'?evidence(edge.source,edge.target):edge.type==='dependencyOf'?evidence(edge.target,edge.source):[];
    return {...edge,status:['partOf','hasPart'].includes(edge.type)?'structural':codeEvidence.length?'source-traced':'catalog-declared',codeEvidence};
  });
  const paths:{entities:string[];direct:boolean;status:string;codeEvidence:unknown[]}[]=[];
  for(const seed of seeds) {
    const walk=(path:string[])=>{
      if(path.length>options.depth)return;
      for(const edge of edges.filter(e=>e.source===path[path.length-1]&&e.type==='dependsOn')) {
        if(path.includes(edge.target))continue;
        if(paths.length>=200){truncated=true;return;}
        const entities=[...path,edge.target];const codeEvidence=evidence(seed,edge.target);
        paths.push({entities,direct:entities.length===2,status:codeEvidence.length?'source-traced':'catalog-declared',codeEvidence});
        walk(entities);
      }
    };
    if(loaded.has(seed))walk([seed]);
  }
  return {nodes:[...loaded].map(([ref,entity])=>({entityRef:ref,kind:entity.kind,name:entity.metadata.name,
    namespace:entity.metadata.namespace??'default',...(indexed.has(ref)?{graphVersion:indexed.get(ref)!.graphVersion}:{})})),edges,dependencyPaths:paths,truncated};
}
