import type { Channel } from "@/shared/api/types";
import { SearchCombobox } from "@/shared/ui/search-combobox";

function formatChannelLabel(ch: Channel): string {
  return `${ch.name} · ${ch.channelType} · ${ch.visibility}`;
}

type ChannelComboboxProps = {
  channels: Channel[];
  disabled?: boolean;
  id?: string;
  onChange: (value: string) => void;
  value: string;
};

export function ChannelCombobox({
  channels,
  disabled,
  id,
  onChange,
  value,
}: ChannelComboboxProps) {
  const selected = channels.find((c) => c.id === value);

  return (
    <SearchCombobox
      disabled={disabled}
      emptyText="No channels found."
      getKey={(c) => c.id}
      id={id}
      items={channels}
      matches={(c, q) =>
        c.name.toLowerCase().includes(q) ||
        (c.channelType?.toLowerCase().includes(q) ?? false) ||
        c.id.toLowerCase().includes(q)
      }
      onSelect={(c) => onChange(c.id)}
      renderItem={(c) => (
        <>
          {c.name}{" "}
          <span className="text-muted-foreground">
            · {c.channelType} · {c.visibility}
          </span>
        </>
      )}
      searchPlaceholder="Search channels..."
      selectedKey={value}
      triggerLabel={
        selected ? formatChannelLabel(selected) : "Select a channel..."
      }
      triggerMuted={!selected}
    />
  );
}
