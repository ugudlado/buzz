import { expect, test } from "@playwright/test";

import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

const MARKETPLACE_AGENT_PUBKEY = TEST_IDENTITIES.alice.pubkey;
const MARKETPLACE_OWNER_PUBKEY = "deadbeef".repeat(8);
const MARKETPLACE_AGENT_EVENT = {
  id: "a".repeat(64),
  pubkey: MARKETPLACE_OWNER_PUBKEY,
  created_at: 1_800_000_000,
  kind: 30177,
  tags: [["d", MARKETPLACE_AGENT_PUBKEY]],
  content: JSON.stringify({
    name: "Rust Reviewer",
    respond_to: "anyone",
    marketplace: {
      listed: true,
      description: "Reviews Rust changes and reports correctness risks",
      capabilities: ["rust", "review"],
      deployment: "remote",
      pricing: { currency: "USD", microunits_per_hour: 12_000_000 },
    },
  }),
  sig: "",
};

test.beforeEach(async ({ page }) => {
  await installMockBridge(page, {
    personaCatalogEvents: [MARKETPLACE_AGENT_EVENT],
    managedAgents: [
      {
        pubkey: MARKETPLACE_AGENT_PUBKEY,
        name: "Rust Reviewer",
        status: "running",
        respondTo: "anyone",
        marketplace: {
          listed: true,
          description: "Reviews Rust changes and reports correctness risks",
          capabilities: ["rust", "review"],
          deployment: "remote",
          pricing: { currency: "USD", microunits_per_hour: 12_000_000 },
        },
      },
      {
        pubkey: TEST_IDENTITIES.bob.pubkey,
        name: "Private Local Agent",
      },
      {
        pubkey: TEST_IDENTITIES.charlie.pubkey,
        name: "Rust Reviewer",
      },
    ],
    relayAgents: [
      {
        pubkey: MARKETPLACE_AGENT_PUBKEY,
        name: "Rust Reviewer",
        status: "online",
      },
    ],
  });
});

async function navigateToWorkflows(page: import("@playwright/test").Page) {
  await page.goto("/");
  await page.getByTestId("open-workflows-view").click();
  await expect(page).toHaveURL(/#\/workflows$/);
  await expect(page.getByTestId("workflows-view")).toBeVisible();
}

async function navigateToMarketplace(page: import("@playwright/test").Page) {
  await page.goto("/");
  await page.getByTestId("open-marketplace-view").click();
  await expect(page).toHaveURL(/#\/marketplace$/);
  await expect(
    page.getByRole("heading", { name: "Marketplace", exact: true }),
  ).toBeVisible();
}

async function createWorkflow(
  page: import("@playwright/test").Page,
  name: string,
  options?: {
    description?: string;
    enabled?: boolean;
    trigger?: string;
    stepCondition?: string;
    stepName?: string;
    stepTimeoutSecs?: string;
  },
) {
  await page.getByRole("button", { name: "Create Workflow" }).click();
  const dialog = page.getByRole("dialog");
  await expect(dialog).toBeVisible();

  await dialog.getByLabel("Workflow name").fill(name);
  if (options?.description) {
    await dialog.getByLabel("Description (optional)").fill(options.description);
  }
  if (options?.enabled === false) {
    await dialog.getByLabel("Workflow is enabled").click();
  }
  if (options?.trigger) {
    await dialog.getByLabel("Trigger").selectOption(options.trigger);
  }

  await dialog.getByRole("button", { name: "Add step" }).click();
  if (options?.stepName) {
    await dialog.getByLabel("Step name (optional)").fill(options.stepName);
  }
  if (options?.stepCondition) {
    await dialog
      .getByLabel("Run condition (optional)")
      .fill(options.stepCondition);
  }
  if (options?.stepTimeoutSecs) {
    await dialog
      .getByLabel("Timeout seconds (optional)")
      .fill(options.stepTimeoutSecs);
  }

  await dialog.getByRole("button", { name: "Create" }).click();

  await expect(
    page.getByRole("heading", { name: "Create Workflow" }),
  ).not.toBeVisible();
}

test("navigates to workflows view and shows empty state", async ({ page }) => {
  await navigateToWorkflows(page);

  await expect(page.getByText("No workflows yet")).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Create your first workflow" }),
  ).toBeVisible();
});

test("discovers a listed agent with presence and sanitized pricing", async ({
  page,
}) => {
  await navigateToMarketplace(page);
  await page
    .getByTestId("workflows-view")
    .getByRole("button", { name: "Agents", exact: true })
    .click();
  await expect(
    page.getByText(
      "Published listings from every community configured in this app.",
    ),
  ).toBeVisible();

  const listing = page.getByTestId(
    `marketplace-agent-${MARKETPLACE_AGENT_PUBKEY}`,
  );
  await expect(listing).toContainText("Rust Reviewer");
  await expect(listing).toContainText("Online");
  await expect(listing).toContainText("USD 12/hour");
  await expect(listing).toContainText("remote");
  await expect(listing).toContainText("Direct use community");
  await expect(listing).not.toContainText("system_prompt");
  await expect(
    listing.getByTestId(`marketplace-agent-pubkey-${MARKETPLACE_AGENT_PUBKEY}`),
  ).toBeVisible();
  const marketplace = page.getByTestId("workflows-view");
  const sameNameUnpublished = marketplace.getByTestId(
    `unpublished-agent-${TEST_IDENTITIES.charlie.pubkey}`,
  );
  await expect(sameNameUnpublished).toContainText("Rust Reviewer");
  const unpublished = marketplace.getByTestId(
    `unpublished-agent-${TEST_IDENTITIES.bob.pubkey}`,
  );
  await expect(unpublished).toContainText("Private Local Agent");
  await unpublished.getByRole("button", { name: "Publish" }).click();
  const publishDialog = page.getByRole("dialog");
  await expect(publishDialog).toContainText("Publish agent listing");
  await publishDialog.getByRole("button", { name: "Cancel" }).click();

  const search = marketplace.getByRole("searchbox", { name: "Search agents" });
  await search.fill("Private Local");
  await expect(unpublished).toBeVisible();
  await expect(listing).toHaveCount(0);
  await search.fill("Rust Reviewer");
  await expect(listing).toBeVisible();
  await expect(unpublished).toHaveCount(0);

  await listing.getByRole("button", { name: "Unpublish" }).click();
  await expect(listing).toHaveCount(0);
  const unpublishedListing = marketplace.getByTestId(
    `unpublished-agent-${MARKETPLACE_AGENT_PUBKEY}`,
  );
  await expect(unpublishedListing).toBeVisible();
  await unpublishedListing.getByRole("button", { name: "Publish" }).click();
  await page
    .getByRole("dialog")
    .getByRole("button", { name: "Publish" })
    .click();
  await expect(listing).toBeVisible();
});

test("searches published workflows by marketplace summary", async ({
  page,
}) => {
  await navigateToWorkflows(page);
  await page.getByRole("button", { name: "Create Workflow" }).click();
  const dialog = page.getByRole("dialog");
  await dialog.getByRole("button", { name: "Edit as YAML" }).click();
  await dialog.locator("textarea").fill(`
name: Release reviewer
trigger:
  on: message_posted
marketplace:
  listed: true
  summary: Reviews release candidates
steps:
  - id: review
    action: assign_to_agent
    agent: Rust Reviewer
    agent_pubkey: ${MARKETPLACE_AGENT_PUBKEY}
    instruction: review it
`);
  await dialog.getByRole("button", { name: "Create" }).click();

  await page.getByTestId("open-marketplace-view").click();
  const search = page.getByRole("searchbox", {
    name: "Search published workflows",
  });
  await search.fill("release candidates");
  await expect(
    page.getByText("Release reviewer", { exact: true }),
  ).toBeVisible();
  await search.fill("no matching workflow");
  await expect(
    page.getByText("No published workflows match your search"),
  ).toBeVisible();
});

test("shows completed and failed assignment receipt evidence", async ({
  page,
}) => {
  await navigateToWorkflows(page);
  await page.getByRole("button", { name: "Create Workflow" }).click();
  const dialog = page.getByRole("dialog");
  await dialog.getByRole("button", { name: "Edit as YAML" }).click();
  await dialog.locator("textarea").fill(`
name: Marketplace review
trigger:
  on: message_posted
marketplace:
  listed: true
  summary: Review a release candidate
  fixed_price:
    currency: USD
    microunits: 5000000
steps:
  - id: completed_review
    action: assign_to_agent
    agent: Rust Reviewer
    agent_pubkey: ${MARKETPLACE_AGENT_PUBKEY}
    instruction: review it
  - id: failed_review
    action: assign_to_agent
    agent: Rust Reviewer
    agent_pubkey: ${MARKETPLACE_AGENT_PUBKEY}
    instruction: fail in e2e
`);
  await dialog.getByRole("button", { name: "Create" }).click();

  const card = page
    .locator('[data-testid^="workflow-card-"]')
    .filter({ hasText: "Marketplace review" });
  await expect(card).toContainText("Listed");
  await expect(card).toContainText("USD 5 fixed display price");
  await card.getByRole("button", { name: "View Marketplace review" }).click();

  const panel = page.getByTestId("workflow-detail-panel");
  await panel.getByRole("button", { name: "Trigger" }).click();
  const receipt = panel.getByTestId("workflow-run-receipt");
  await expect(receipt).toContainText("Accounting preview");
  await expect(receipt).toContainText("Fixed display price");
  await expect(receipt).toContainText("USD 5");
  await expect(receipt).toContainText("USD 0.006666");

  const assignments = panel.getByTestId("assignment-receipt");
  await expect(assignments).toHaveCount(2);
  await expect(assignments.nth(0)).toContainText("completed");
  await expect(assignments.nth(0)).toContainText("not required");
  await expect(assignments.nth(1)).toContainText("failed");
  await expect(assignments.nth(1)).toContainText("human review required");
  await expect(assignments.nth(1)).toContainText("Completion ID unavailable");
  await expect(assignments.nth(1)).toContainText("Live telemetry: unavailable");
  await expect(assignments.nth(1)).toContainText(
    "Usage diagnostics: unavailable",
  );

  const reportedUsage = assignments.nth(0).getByTestId("reported-usage");
  await expect(reportedUsage).toContainText("Reported usage");
  await expect(reportedUsage.getByTestId("reported-usage-model")).toHaveText(
    "claude-sonnet-5",
  );
  await expect(reportedUsage.getByTestId("reported-usage-tokens")).toHaveText(
    "15,500 in / 2,000 out",
  );
  await expect(reportedUsage.getByTestId("reported-usage-cost")).toHaveText(
    "USD 0.042",
  );
  await expect(reportedUsage).toContainText("self-reported");

  await expect(assignments.nth(1).getByTestId("reported-usage")).toHaveText(
    /Reported usage\s*None/,
  );
});

test("creates a workflow via the form builder", async ({ page }) => {
  const workflowName = `test_workflow_${Date.now()}`;

  await navigateToWorkflows(page);
  await createWorkflow(page, workflowName);

  // Verify workflow appears in the list
  await expect(page.getByTestId("workflows-view")).toContainText(workflowName);
});

test("disables autocapitalization in the workflow form", async ({ page }) => {
  await navigateToWorkflows(page);

  await page.getByRole("button", { name: "Create Workflow" }).click();
  const dialog = page.getByRole("dialog");

  await expect(dialog.getByLabel("Workflow name")).toHaveAttribute(
    "autocapitalize",
    "off",
  );

  await dialog.getByRole("button", { name: "Add step" }).click();
  await expect(dialog.getByLabel("Step name (optional)")).toHaveAttribute(
    "autocapitalize",
    "off",
  );
});

test("captures disabled diff workflows in the list UI", async ({ page }) => {
  const workflowName = `diff_workflow_${Date.now()}`;
  const description = "Watches diff events for src/ changes";

  await navigateToWorkflows(page);
  await createWorkflow(page, workflowName, {
    description,
    enabled: false,
    trigger: "diff_posted",
    stepName: "Notify reviewers",
    stepCondition: 'str_contains(trigger_text, "src/")',
    stepTimeoutSecs: "45",
  });

  const card = page
    .locator('[data-testid^="workflow-card-"]')
    .filter({ hasText: workflowName })
    .first();
  await expect(card).toContainText(workflowName);
  await expect(card).toContainText(description);
  await expect(card).toContainText("Diff Posted");
  await expect(card).toContainText("disabled");
});

test("shows the webhook secret dialog after saving a webhook workflow", async ({
  page,
}) => {
  const workflowName = `webhook_workflow_${Date.now()}`;

  await navigateToWorkflows(page);
  await createWorkflow(page, workflowName, {
    trigger: "webhook",
  });

  await expect(page.getByText("Webhook Ready")).toBeVisible();
  await expect(page.getByRole("button", { name: "Copy URL" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Copy Secret" })).toBeVisible();

  await page.getByRole("button", { name: "Close" }).click();
  await expect(page.getByText("Webhook Ready")).not.toBeVisible();
});

test("edits an existing workflow", async ({ page }) => {
  const originalName = `edit_test_${Date.now()}`;
  const updatedName = `${originalName}_updated`;

  await navigateToWorkflows(page);
  await createWorkflow(page, originalName);

  // Verify it exists
  await expect(page.getByTestId("workflows-view")).toContainText(originalName);

  // Open the dropdown menu and click Edit
  await page.getByRole("button", { name: "Workflow actions" }).first().click();
  await page.getByRole("menuitem", { name: "Edit" }).click();

  // Dialog should open in edit mode
  await expect(page.getByRole("dialog")).toBeVisible();
  await expect(page.getByText("Edit Workflow")).toBeVisible();

  // Change the name
  const nameInput = page.getByLabel("Workflow name");
  await nameInput.clear();
  await nameInput.fill(updatedName);

  // Save
  await page.getByRole("button", { name: "Save" }).click();
  await expect(page.getByRole("dialog")).not.toBeVisible();

  // Verify the updated name appears
  await expect(page.getByTestId("workflows-view")).toContainText(updatedName);
});

test("duplicates a workflow", async ({ page }) => {
  const originalName = `dup_test_${Date.now()}`;

  await navigateToWorkflows(page);
  await createWorkflow(page, originalName);

  // Open the dropdown menu and click Duplicate
  await page.getByRole("button", { name: "Workflow actions" }).first().click();
  await page.getByRole("menuitem", { name: "Duplicate" }).click();

  // Dialog should open in duplicate mode with "(copy)" suffix
  await expect(page.getByRole("dialog")).toBeVisible();
  await expect(page.getByText("Duplicate Workflow")).toBeVisible();

  // Submit the duplicate
  await page.getByRole("button", { name: "Create Copy" }).click();
  await expect(page.getByRole("dialog")).not.toBeVisible();

  // Both the original and copy should exist
  await expect(page.getByTestId("workflows-view")).toContainText(originalName);
});

test("deletes a workflow with confirmation", async ({ page }) => {
  const workflowName = `delete_test_${Date.now()}`;

  await navigateToWorkflows(page);
  await createWorkflow(page, workflowName);

  // Verify it exists
  await expect(page.getByTestId("workflows-view")).toContainText(workflowName);

  // Open the dropdown menu and click Delete
  await page.getByRole("button", { name: "Workflow actions" }).first().click();
  await page.getByRole("menuitem", { name: "Delete" }).click();

  // Confirmation dialog should appear with workflow name
  await expect(page.getByRole("alertdialog")).toBeVisible();
  await expect(page.getByRole("alertdialog")).toContainText(workflowName);

  // Confirm deletion
  await page.getByRole("button", { name: "Delete" }).click();
  await expect(page.getByRole("alertdialog")).not.toBeVisible();

  // Verify workflow is gone — back to empty state
  await expect(page.getByText("No workflows yet")).toBeVisible();
});

test("triggers a workflow from the detail panel", async ({ page }) => {
  const workflowName = `trigger_test_${Date.now()}`;

  await navigateToWorkflows(page);
  await createWorkflow(page, workflowName);

  // Click on the workflow card to open the detail panel
  await page.getByRole("button", { name: `View ${workflowName}` }).click();
  await expect(page.getByTestId("workflow-detail-panel")).toBeVisible();

  // Click the Trigger button
  await page
    .getByTestId("workflow-detail-panel")
    .getByRole("button", { name: "Trigger" })
    .click();

  // Wait for the trigger to complete (button text changes back from "Triggering...")
  await expect(
    page
      .getByTestId("workflow-detail-panel")
      .getByRole("button", { name: "Trigger" }),
  ).toBeVisible();

  await expect(
    page
      .getByTestId("workflow-detail-panel")
      .getByTestId("workflow-selected-run"),
  ).toBeVisible();
  await expect(
    page.getByTestId("workflow-detail-panel").getByTestId("workflow-run-trace"),
  ).toContainText("step_1");
});
