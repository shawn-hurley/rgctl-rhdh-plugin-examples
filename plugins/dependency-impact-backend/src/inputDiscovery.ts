import { demoInputs } from './demoInputs';

export interface ComparisonInput {
  id: string;
  label: string;
  description: string;
  packageManager: 'maven';
  entityRef: string;
  applicationRevision: string;
  dependency: {
    groupId: string;
    artifactId: string;
    type: 'jar';
    classifier: null;
  };
  /** Advertised by discovery; analysis independently resolves the actual baseline. */
  baselineVersion: string;
  targetVersion: string;
  strategy: 'direct-upgrade' | 'transitive-override';
}

export interface DiscoveredInputs {
  schemaVersion: '1';
  provider: string;
  inputs: ComparisonInput[];
}

/** Discovery supplies choices; analysis and artifact acquisition stay in Rust. */
export interface InputDiscoveryService {
  discoverInputs(options: { entityRef?: string }): Promise<DiscoveredInputs>;
}

export class DemoInputDiscoveryService implements InputDiscoveryService {
  async discoverInputs({ entityRef }: { entityRef?: string }): Promise<DiscoveredInputs> {
    return {
      schemaVersion: '1',
      provider: 'demo-manifest',
      inputs: structuredClone(demoInputs.filter(input => !entityRef || input.entityRef === entityRef)),
    };
  }
}
