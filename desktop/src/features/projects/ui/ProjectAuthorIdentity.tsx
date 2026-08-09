import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { UserProfilePopover } from "@/features/profile/ui/UserProfilePopover";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import { UserAvatar } from "@/shared/ui/UserAvatar";

/** Compact work-item author identity with a minimal hover summary.
 * `pubkey: null` renders `label` as plain text with no avatar lookup or
 * profile popover — for external (GitHub/Backlog) authors that aren't a
 * real Nostr pubkey. */
export function ProjectAuthorIdentity({
  label,
  profiles,
  pubkey,
  testId,
}: {
  label: string;
  profiles?: UserProfileLookup;
  pubkey: string | null;
  testId?: string;
}) {
  const profile = pubkey ? profiles?.[normalizePubkey(pubkey)] : undefined;
  const roleLabel = profile?.isAgent === true ? "Agent" : "Person";

  const avatar = (
    <UserAvatar
      accent={profile?.isAgent === true}
      avatarUrl={profile?.avatarUrl ?? null}
      displayName={label}
      fallbackDelayMs={0}
      size="xs"
      testId={testId ? `${testId}-avatar` : undefined}
    />
  );

  if (!pubkey) {
    return (
      <span
        className="inline-flex items-center gap-1 align-middle"
        data-testid={testId}
      >
        {avatar}
        <span data-testid={testId ? `${testId}-label` : undefined}>
          {label}
        </span>
      </span>
    );
  }

  return (
    <span className="inline-flex align-middle">
      <UserProfilePopover
        enableHoverPopover={false}
        pubkey={pubkey}
        triggerElement="span"
      >
        <Tooltip>
          <TooltipTrigger asChild>
            <button
              className="relative z-10 inline-flex items-center gap-1 rounded-sm hover:underline focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring"
              data-testid={testId}
              type="button"
            >
              {avatar}
              <span data-testid={testId ? `${testId}-label` : undefined}>
                {label}
              </span>
            </button>
          </TooltipTrigger>
          <TooltipContent
            className="flex items-center gap-2 px-2.5 py-2"
            data-testid={testId ? `${testId}-rollover` : undefined}
            side="top"
          >
            <UserAvatar
              accent={profile?.isAgent === true}
              avatarUrl={profile?.avatarUrl ?? null}
              displayName={label}
              fallbackDelayMs={0}
              size="sm"
            />
            <span className="min-w-0">
              <span className="block truncate font-medium">{label}</span>
              <span className="block text-primary-foreground/70">
                {roleLabel}
              </span>
            </span>
          </TooltipContent>
        </Tooltip>
      </UserProfilePopover>
    </span>
  );
}
