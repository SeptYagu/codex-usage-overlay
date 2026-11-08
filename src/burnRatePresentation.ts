import { TFunction } from 'i18next';
import { CodexUsage } from './types';

export function burnRatePresentation(usage: CodexUsage | null | undefined, t:TFunction): string {
  if(!usage?.effectiveAlgorithm)return t('burnUnavailable');
  const parts=[t('burnUsing',{algorithm:t(`burnAlgorithm_${usage.effectiveAlgorithm}`)})];
  if(usage.fallbackReason)parts.push(t(usage.fallbackReason==='identityUnavailable'?'burnIdentityUnavailable':'burnFallback'));
  if(usage.requestedAlgorithm==='personalized')parts.push(t(`burnLearning_${usage.personalLearningStatus??'accumulating'}`));
  return parts.join(' · ');
}
