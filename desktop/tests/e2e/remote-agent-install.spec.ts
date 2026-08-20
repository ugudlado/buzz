import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";
import { waitForAnimations } from "../helpers/animations";

const REMOTE_AGENT_PUBKEY = "ab".repeat(32);
const REMOTE_OWNER_PUBKEY = "cd".repeat(32);
const RELAY_SELF = "ee".repeat(32);
const REMOTE_COMMUNITY = {
  id: "remote-community-a",
  name: "Community A",
  relayUrl: "ws://localhost:39997",
  addedAt: "2026-01-02T00:00:00.000Z",
};

const REMOTE_LISTING_EVENT = {
  id: "b".repeat(64),
  pubkey: REMOTE_OWNER_PUBKEY,
  created_at: 1_800_000_000,
  kind: 30177,
  tags: [["d", REMOTE_AGENT_PUBKEY]],
  content: JSON.stringify({
    name: "Remote Reviewer",
    respond_to: "anyone",
    marketplace: {
      listed: true,
      description: "Reviews changes from its home community",
      capabilities: ["rust", "review"],
      deployment: "remote",
      pricing: { currency: "USD", microunits_per_hour: 12_000_000 },
      remote_invocation: { policy: "any_community" },
    },
  }),
  sig: "",
};

test.beforeEach(async ({ page }) => {
  await installMockBridge(page, {
    personaCatalogEvents: [REMOTE_LISTING_EVENT],
    relaySelf: RELAY_SELF,
  });
  // Add a second community after the bridge's default seeding so the
  // listing also surfaces as a *remote* marketplace entry.
  await page.addInitScript((remote) => {
    const raw = window.localStorage.getItem("buzz-communities");
    const list: unknown[] = raw ? JSON.parse(raw) : [];
    list.push(remote);
    window.localStorage.setItem("buzz-communities", JSON.stringify(list));
  }, REMOTE_COMMUNITY);
});

function remoteCard(page: import("@playwright/test").Page) {
  return page
    .getByTestId(`marketplace-agent-${REMOTE_AGENT_PUBKEY}`)
    .filter({ hasText: "Community A" });
}

async function navigateToMarketplace(page: import("@playwright/test").Page) {
  await page.goto("/");
  await page.getByTestId("open-marketplace-view").click();
  await expect(page).toHaveURL(/#\/marketplace$/);
  await expect(
    page.getByRole("heading", { name: "Marketplace", exact: true }),
  ).toBeVisible();
  await page
    .getByTestId("workflows-view")
    .getByRole("button", { name: "Agents", exact: true })
    .click();
}

async function installRemoteAgent(page: import("@playwright/test").Page) {
  const card = remoteCard(page);
  await expect(card).toBeVisible();
  await card.getByRole("button", { name: "Add to community" }).click();
  const dialog = page.getByRole("dialog");
  await expect(
    dialog.getByRole("heading", {
      name: "Add Remote Reviewer to this community",
    }),
  ).toBeVisible();
  await dialog.getByRole("button", { name: "Add to community" }).click();
  await expect(dialog).not.toBeVisible();
  await expect(card.getByRole("button", { name: "Ask" })).toBeVisible();
}

test("installs a remote agent instead of opening the workflow editor", async ({
  page,
}) => {
  await navigateToMarketplace(page);

  const card = remoteCard(page);
  await expect(card).toBeVisible();
  await expect(card.getByText("USD 12/hour")).toBeVisible();
  await expect(card.getByText("Remote-ready")).toBeVisible();
  await expect(card.getByText("Community A")).toBeVisible();
  // The one-shot "Use here" flow is gone.
  await expect(
    card.getByRole("button", { name: "Use here" }),
  ).not.toBeVisible();

  await installRemoteAgent(page);

  await expect(card.getByText("Added")).toBeVisible();
  await expect(card.getByRole("button", { name: "Runs" })).toBeVisible();
  await waitForAnimations(page);
  await card.screenshot({
    path: "test-results/remote-agent/01-installed-card.png",
  });
  await expect(
    card.getByRole("button", { name: "Remove from community" }),
  ).toBeVisible();

  // The install created the hidden workflow definition, not a visible one.
  const created = await page.evaluate(() =>
    (window.__BUZZ_E2E_COMMAND_PAYLOADS__ ?? []).filter(
      (entry) => entry.command === "create_workflow",
    ),
  );
  expect(created).toHaveLength(1);
  const yaml = (created[0].payload as { yamlDefinition: string })
    .yamlDefinition;
  expect(yaml).toContain("installed_agent: true");
  expect(yaml).toContain("{{trigger.prompt}}");
  expect(yaml).toContain(`agent_relay_pubkey: ${RELAY_SELF}`);
  expect(yaml).toContain("{{steps.ask.output.result}}");

  // The hidden workflow stays out of the workflows list.
  await page.getByTestId("open-workflows-view").click();
  await expect(page).toHaveURL(/#\/workflows$/);
  await expect(page.getByText("No workflows yet")).toBeVisible();
});

test("asks an installed remote agent with a prompt", async ({ page }) => {
  await navigateToMarketplace(page);
  await installRemoteAgent(page);

  await remoteCard(page).getByRole("button", { name: "Ask" }).click();
  const dialog = page.getByRole("dialog");
  await expect(
    dialog.getByRole("heading", { name: "Ask Remote Reviewer" }),
  ).toBeVisible();
  const send = dialog.getByRole("button", { name: "Send" });
  await expect(send).toBeDisabled();
  await dialog
    .getByRole("textbox", { name: "Prompt" })
    .fill("Review the release notes");
  await waitForAnimations(page);
  await dialog.screenshot({
    path: "test-results/remote-agent/02-ask-dialog.png",
  });
  await send.click();
  await expect(dialog).not.toBeVisible();

  const triggers = await page.evaluate(() =>
    (window.__BUZZ_E2E_COMMAND_PAYLOADS__ ?? []).filter(
      (entry) => entry.command === "trigger_workflow",
    ),
  );
  expect(triggers).toHaveLength(1);
  expect(
    (triggers[0].payload as { fields: { prompt: string } }).fields,
  ).toEqual({ prompt: "Review the release notes" });

  // The run is reachable from the agent card via "Runs".
  await remoteCard(page).getByRole("button", { name: "Runs" }).click();
  const detail = page.getByTestId("workflow-detail-panel");
  await expect(detail).toBeVisible();
  await expect(detail.getByText("Remote Reviewer").first()).toBeVisible();
  await waitForAnimations(page);
  await detail.screenshot({
    path: "test-results/remote-agent/03-runs-panel.png",
  });
});
