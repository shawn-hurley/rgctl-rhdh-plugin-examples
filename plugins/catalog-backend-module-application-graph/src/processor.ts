import type {Entity} from '@backstage/catalog-model';
import type {CatalogProcessor,CatalogProcessorEmit} from '@backstage/plugin-catalog-node';
import {processingResult} from '@backstage/plugin-catalog-node';
import type {LocationSpec} from '@backstage/plugin-catalog-common';
import {GraphClient,onboardingRequest} from '@internal/backstage-plugin-application-graph-backend';
import {enrich,Manifest,ref} from './enrichment';

interface Pending {fingerprint:string;submitted:number;id:string;state:string}
export class ApplicationGraphProcessor implements CatalogProcessor {
  private readonly pending=new Map<string,Pending>();
  constructor(private readonly options:{service:Pick<GraphClient,'request'>;readEntities:()=>Promise<Entity[]>;refreshSeconds?:number;
    warn:(message:string)=>void}) {}
  getProcessorName(){return 'ApplicationGraphProcessor';}
  getPriority(){return 10;}
  async preProcessEntity(entity:Entity,location:LocationSpec,emit:CatalogProcessorEmit):Promise<Entity> {
    if(entity.kind.toLowerCase()!=='component'||entity.metadata.annotations?.['application-graph.io/enabled']!=='true')return entity;
    const reference=ref(entity);
    let status='pending';
    try {
      const request=onboardingRequest(entity,reference);
      const fingerprint=JSON.stringify(request.source);
      let pending=this.pending.get(reference);
      const age=Date.now()-(pending?.submitted??0);
      if(!pending||pending.fingerprint!==fingerprint||
        (pending.state==='succeeded'&&age>=(this.options.refreshSeconds??3600)*1000)||
        (pending.state==='failed'&&age>=60000)) {
        const job=await this.options.service.request<{id:string;state:string}>('/v1/onboard',request);
        pending={fingerprint,submitted:Date.now(),id:job.id,state:job.state};this.pending.set(reference,pending);
      }
      const job=await this.options.service.request<{state:string}>(`/v1/jobs/${pending.id}`);
      pending.state=job.state;status=job.state;
      if(job.state!=='succeeded')return this.annotate(entity,status);
      const data=await this.options.service.request<{applications:Manifest[]}>('/v1/applications');
      const own=data.applications.find(a=>a.entityRef===reference&&JSON.stringify(a.source)===fingerprint);
      if(!own)return this.annotate(entity,'pending');
      const {entity:enriched,api}=enrich(entity,own,data.applications,await this.options.readEntities());
      if(api)emit(processingResult.entity(location,api));
      return this.annotate(enriched,'indexed',own.graphVersion);
    } catch(error) {
      // A source/graph outage must not prevent the descriptor from being imported.
      this.options.warn(`Application graph processing failed for ${reference}: ${error instanceof Error?error.message:'unknown error'}`);
      return this.annotate(entity,'failed');
    }
  }
  private annotate(entity:Entity,status:string,version?:string):Entity {
    return {...entity,metadata:{...entity.metadata,annotations:{...entity.metadata.annotations,
      'application-graph.io/status':status,...(version?{'application-graph.io/graph-version':version}:{})}}};
  }
}
