export type WorkflowStatus = "active" | "disabled" | "archived";

export type Workflow = {
  id: string;
  name: string;
  ownerPubkey: string;
  channelId: string | null;
  definition: Record<string, unknown>;
  status: WorkflowStatus;
  createdAt: number;
  updatedAt: number;
};

export type WorkflowSaveResult = {
  workflow: Workflow;
  webhookSecret: string | null;
};

export type WorkflowRunStatus =
  | "pending"
  | "running"
  | "completed"
  | "failed"
  | "timed_out"
  | "cancelled"
  | "waiting_approval"
  | "waiting_agent";

export type TraceEntry = {
  stepId: string;
  status: string;
  output: Record<string, unknown>;
  startedAt: number | null;
  completedAt: number | null;
  error: string | null;
  assignmentReceipt: AssignmentReceipt | null;
};

export type AssignmentOutcome =
  | "pending"
  | "completed"
  | "failed"
  | "timed_out"
  | "cancelled"
  | "not_started";

export type AssignmentReviewState = "not_required" | "human_review_required";

export type AssignmentReceipt = {
  agentPubkey: string;
  agentOwnerPubkey: string | null;
  promptEventId: string;
  completionEventId: string | null;
  promptPublishedAtMs: number | null;
  terminalAtMs: number | null;
  durationMs: number | null;
  rateCurrency: string | null;
  rateMicrounitsPerHour: number | null;
  estimatedMicrounits: number | null;
  outcome: AssignmentOutcome;
  reviewState: AssignmentReviewState;
};

export type WorkflowRun = {
  id: string;
  workflowId: string;
  status: WorkflowRunStatus;
  currentStep: number | null;
  executionTrace: TraceEntry[];
  startedAt: number | null;
  completedAt: number | null;
  errorMessage: string | null;
  workflowAuthorPubkey: string | null;
  fixedPriceCurrency: string | null;
  fixedPriceMicrounits: number | null;
  createdAt: number;
};

export type WorkflowApprovalStatus =
  | "pending"
  | "granted"
  | "denied"
  | "expired";

export type WorkflowApproval = {
  token: string;
  workflowId: string;
  runId: string;
  stepId: string;
  stepIndex: number;
  approverSpec: string;
  status: WorkflowApprovalStatus;
  approverPubkey: string | null;
  note: string | null;
  expiresAt: string;
  createdAt: number;
};

export type TriggerWorkflowResponse = {
  runId: string;
  workflowId: string;
  status: string;
};

export type ApprovalActionResponse = {
  token: string;
  status: string;
  runId: string;
  workflowId: string;
};
