import { useChannelMembersQuery } from "@/features/channels/hooks";
import type { ChannelMember } from "@/shared/api/types";
import { SearchCombobox } from "@/shared/ui/search-combobox";

function memberLabel(member: ChannelMember): string {
  return member.displayName?.trim() || member.pubkey.slice(0, 12);
}

type AgentComboboxProps = {
  channelId: string | null;
  disabled?: boolean;
  id?: string;
  onChange: (agent: { displayName: string; pubkey: string }) => void;
  value: string;
};

/** Searchable picker over a channel's members, for `assign_to_agent`'s
 * `agent`/`agent_pubkey` fields. Falls back to a plain text input (via the
 * caller) when `channelId` is unset — e.g. before the workflow's target
 * channel has been chosen. */
export function AgentCombobox({
  channelId,
  disabled,
  id,
  onChange,
  value,
}: AgentComboboxProps) {
  const membersQuery = useChannelMembersQuery(channelId);
  const members = membersQuery.data ?? [];

  const selected = members.find(
    (m) => memberLabel(m).toLowerCase() === value.trim().toLowerCase(),
  );

  return (
    <SearchCombobox
      disabled={disabled || !channelId}
      emptyText="No members found."
      getKey={(m) => m.pubkey}
      id={id}
      items={members}
      loading={membersQuery.isLoading}
      loadingText="Loading members…"
      matches={(m, q) =>
        memberLabel(m).toLowerCase().includes(q) ||
        m.pubkey.toLowerCase().includes(q)
      }
      onSelect={(m) =>
        onChange({ displayName: memberLabel(m), pubkey: m.pubkey })
      }
      renderItem={(m) => (
        <>
          {memberLabel(m)}{" "}
          {m.isAgent ? (
            <span className="text-muted-foreground">· agent</span>
          ) : null}
        </>
      )}
      searchPlaceholder="Search members..."
      selectedKey={selected?.pubkey}
      triggerLabel={
        value ||
        (channelId ? "Select a channel member..." : "Pick a channel first")
      }
      triggerMuted={!value}
    />
  );
}
