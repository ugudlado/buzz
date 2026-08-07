import type { ProjectPullRequest } from "@/features/projects/hooks";
import { ClearFiltersChip, FilterChip } from "@/shared/ui/filter-chip-bar";

export const PROJECT_PULL_REQUEST_STATUSES: ProjectPullRequest["status"][] = [
  "Open",
  "Draft",
  "Merged",
  "Closed",
];

/** Filter bar shown above the flat pull-request list: status, label, and
 * author chips, plus a "Clear filters" affordance when any filter is
 * active. Intentionally basic client-side filtering — no URL persistence
 * or grouping. */
export function PullRequestsFilterBar({
  authorOptions,
  authors,
  labelOptions,
  labels,
  onAuthorsChange,
  onClear,
  onLabelsChange,
  onStatusesChange,
  statuses,
}: {
  authorOptions: Array<{ label: string; value: string }>;
  authors: string[];
  labelOptions: Array<{ label: string; value: string }>;
  labels: string[];
  onAuthorsChange: (value: string[]) => void;
  onClear: () => void;
  onLabelsChange: (value: string[]) => void;
  onStatusesChange: (value: ProjectPullRequest["status"][]) => void;
  statuses: ProjectPullRequest["status"][];
}) {
  const hasActiveFilters =
    statuses.length > 0 || labels.length > 0 || authors.length > 0;

  return (
    <div className="flex flex-wrap items-center gap-1.5 border-b border-border/50 px-4 py-2.5">
      <FilterChip
        label="Status"
        onChange={onStatusesChange}
        options={PROJECT_PULL_REQUEST_STATUSES.map((status) => ({
          label: status,
          value: status,
        }))}
        testId="project-pull-requests-filter-status"
        value={statuses}
      />
      {labelOptions.length > 0 ? (
        <FilterChip
          label="Label"
          onChange={onLabelsChange}
          options={labelOptions}
          testId="project-pull-requests-filter-label"
          value={labels}
        />
      ) : null}
      {authorOptions.length > 0 ? (
        <FilterChip
          label="Author"
          onChange={onAuthorsChange}
          options={authorOptions}
          testId="project-pull-requests-filter-author"
          value={authors}
        />
      ) : null}
      {hasActiveFilters ? (
        <ClearFiltersChip
          onClear={onClear}
          testId="project-pull-requests-filter-clear"
        />
      ) : null}
    </div>
  );
}
