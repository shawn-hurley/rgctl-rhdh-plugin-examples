import { coreServices, createBackendPlugin } from '@backstage/backend-plugin-api';
import { GraphClient } from './client';
import { Entity, HttpError } from './catalog';
import { createRouter } from './router';

export default createBackendPlugin({
  pluginId: 'application-graph',
  register(env) {
    env.registerInit({
      deps: {httpRouter:coreServices.httpRouter, config:coreServices.rootConfig, logger:coreServices.logger,
        discovery:coreServices.discovery, auth:coreServices.auth, httpAuth:coreServices.httpAuth},
      async init({httpRouter,config,logger,discovery,auth,httpAuth}) {
        const service = new GraphClient(config.getOptionalString('applicationGraph.service.baseUrl'),config.getOptionalString('applicationGraph.service.token'));
        const router = createRouter({service,readEntity:async (request,ref) => {
          const credentials = await httpAuth.credentials(request);
          const {token} = await auth.getPluginRequestToken({onBehalfOf:credentials,targetPluginId:'catalog'});
          const [kind,namespace,name] = ref.split(/[:/]/);
          const base = await discovery.getBaseUrl('catalog');
          let response: globalThis.Response;
          try { response = await fetch(`${base}/entities/by-name/${kind}/${namespace}/${name}`, {headers:{Authorization:`Bearer ${token}`},signal:AbortSignal.timeout(15000)}); }
          catch { throw new HttpError(502,'CATALOG_UNAVAILABLE','Catalog is unavailable'); }
          if (!response.ok) throw new HttpError([403,404].includes(response.status)?response.status:502,'ENTITY_UNAVAILABLE','Catalog entity is unavailable or not authorized');
          return await response.json() as Entity;
        }});
        httpRouter.use(router);
        logger.info('Application graph onboarding backend loaded');
      },
    });
  },
});
