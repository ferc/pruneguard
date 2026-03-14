import { ARTIFACT_TYPES } from '@artifacts/constants';
import { ReportTable } from '@reports/components';
import { formatExperiment } from '@experiment-management-shared/utils';

export function app() {
  return { ARTIFACT_TYPES, ReportTable, formatExperiment };
}
