import { describe, expect, it } from 'vitest';
import type { ConfigControl } from '@acpira/shared/transcript';
import { modelControlsOf } from '../settings/AgentPage';

const opt = (id: string, name = id) => ({ id, name });

describe('modelControlsOf', () => {
  it('excludes reasoning, booleans, approval levels, Fast selects and sidekick controls while keeping models, collaboration and custom agents', () => {
    const controls: ConfigControl[] = [
      { id: 'model', name: 'Model', category: 'model', value: 'opus', options: [opt('opus', 'Opus 5.5')] },
      { id: 'effort', name: 'Effort', category: 'thought_level', value: 'high', options: [opt('low'), opt('high')] },
      { id: 'fast', name: 'Fast mode', category: 'speed', type: 'boolean', value: 'false', options: [opt('false', 'Off'), opt('true', 'On')] },
      { id: 'speed', name: 'Speed', category: 'model_config', value: 'standard', options: [opt('standard', 'Standard'), opt('fast', 'Fast')] },
      { id: 'approval', name: 'Approval', category: '_permission_mode', value: 'full', options: [opt('ask', 'Manual'), opt('full', 'Bypass')] },
      { id: 'collaboration_mode', name: 'Collaboration mode', category: 'collaboration_mode', value: 'default', options: [opt('default', 'Default'), opt('plan', 'Plan')] },
      { id: 'agent', name: 'Agent', value: 'default', options: [opt('default', 'Default'), opt('reviewer', 'reviewer')] },
      { id: 'sidekick', name: 'Sidekick', category: '_sidekick', value: 'none', options: [opt('none', 'None'), opt('opus', 'Opus 5.5')] },
      { id: 'sidekick_effort', name: 'Sidekick effort', category: '_sidekick_effort', value: 'high', options: [opt('low'), opt('high')] },
      { id: 'sidekick_fast', name: 'Sidekick fast', category: '_sidekick_fast', type: 'boolean', value: 'false', options: [opt('false', 'Off'), opt('true', 'On')] },
    ];
    expect(modelControlsOf(controls)?.map(c => c.id)).toEqual(['model', 'collaboration_mode', 'agent']);
  });
});
