import {coreServices,createBackendModule} from '@backstage/backend-plugin-api';
import type {Entity} from '@backstage/catalog-model';
import {catalogProcessingExtensionPoint} from '@backstage/plugin-catalog-node/alpha';
import {GraphClient} from '@internal/backstage-plugin-application-graph-backend';
import {ApplicationGraphProcessor} from './processor';

export default createBackendModule({pluginId:'catalog',moduleId:'application-graph',register(env) {
  env.registerInit({deps:{catalog:catalogProcessingExtensionPoint,config:coreServices.rootConfig,
    discovery:coreServices.discovery,auth:coreServices.auth,logger:coreServices.logger},
    async init({catalog,config,discovery,auth,logger}) {
      const service=new GraphClient(config.getOptionalString('applicationGraph.service.baseUrl'),config.getOptionalString('applicationGraph.service.token'));
      catalog.addProcessor(new ApplicationGraphProcessor({service,
        refreshSeconds:config.getOptionalNumber('applicationGraph.catalog.refreshSeconds'),warn:message=>logger.warn(message),
        readEntities:async()=>{
          const {token}=await auth.getPluginRequestToken({onBehalfOf:await auth.getOwnServiceCredentials(),targetPluginId:'catalog'});
          const base=await discovery.getBaseUrl('catalog');
          const entities:Entity[]=[];let cursor:string|undefined;
          do {
            const query=new URLSearchParams(cursor?{cursor}:{limit:'200'});
            if(!cursor){query.append('filter','kind=component');query.append('filter','kind=resource');}
            const response=await fetch(`${base}/entities/by-query?${query}`,{headers:{Authorization:`Bearer ${token}`},signal:AbortSignal.timeout(15000)});
            if(!response.ok)throw new Error('Cannot read catalog entities for graph enrichment');
            const data=await response.json() as {items:Entity[];pageInfo?:{nextCursor?:string}};
            entities.push(...data.items);cursor=data.pageInfo?.nextCursor;
            if(entities.length>10000)throw new Error('Catalog enrichment entity limit exceeded');
          }while(cursor);
          return entities;
        }}));
      logger.info('Application graph catalog processor loaded');
    }});
}});
