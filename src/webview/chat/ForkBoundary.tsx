import { memo, useContext } from 'react';
import { ArrowRight, GitBranch } from 'lucide-react';
import type { AgentId, AgentInfo, ForkOrigin } from '@shared/transcript';
import { t } from '../i18n';
import { Row, RowLabel, RowTarget } from '../ui/Row';
import { AgentMark } from './AgentMark';
import { OpenSessionContext, SessionTitleContext } from './fileLinks';

// The row under a fork's copied turns, between two exchanges: the session they were copied from (a click opens it) and,
// for a fork onto another agent, the agent that wrote them and the one that continues
export const ForkBoundary = memo(function ForkBoundary({ origin, agents, agentId }: { origin: ForkOrigin; agents: AgentInfo[]; agentId: AgentId }) {
  const openSession = useContext(OpenSessionContext);
  const titleOf = useContext(SessionTitleContext);
  const name = (id: AgentId) => agents.find(a => a.id === id)?.name ?? id;
  const title = titleOf?.(origin.sessionId);
  const across = origin.agent && origin.agent !== agentId ? origin.agent : undefined;
  return (
    <Row data-fork-boundary as={openSession ? 'button' : 'div'} interactive={!!openSession} lead={<GitBranch strokeWidth={1.5} />}
      onClick={openSession ? () => openSession(origin.sessionId) : undefined}
      trailing={across && (
        <span className="flex items-center gap-1" title={`${name(across)} → ${name(agentId)}`}>
          <AgentMark id={across} name={name(across)} />
          <ArrowRight className="size-icon" strokeWidth={1.5} aria-hidden />
          <AgentMark id={agentId} name={name(agentId)} />
        </span>
      )}>
      <RowLabel>{t('turn.forkedFrom')}</RowLabel>
      <RowTarget>{title ?? t('session.fallbackTitle')}</RowTarget>
    </Row>
  );
});
