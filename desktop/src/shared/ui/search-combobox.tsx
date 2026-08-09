import { Check, ChevronsUpDown, Search } from "lucide-react";
import * as React from "react";

import { cn } from "@/shared/lib/cn";
import { Popover, PopoverContent, PopoverTrigger } from "@/shared/ui/popover";

type SearchComboboxProps<T> = {
  disabled?: boolean;
  emptyText: string;
  getKey: (item: T) => string;
  id?: string;
  items: T[];
  loading?: boolean;
  loadingText?: string;
  matches: (item: T, query: string) => boolean;
  onSelect: (item: T) => void;
  renderItem: (item: T) => React.ReactNode;
  searchPlaceholder: string;
  selectedKey?: string;
  triggerLabel: React.ReactNode;
  /** Muted trigger styling when nothing is selected yet. */
  triggerMuted?: boolean;
};

/** Generic searchable single-select combobox: trigger button + popover with
 * a filter input and keyboard-navigable list. Shared skeleton of
 * `ChannelCombobox` and `AgentCombobox`. */
export function SearchCombobox<T>({
  disabled,
  emptyText,
  getKey,
  id,
  items,
  loading,
  loadingText,
  matches,
  onSelect,
  renderItem,
  searchPlaceholder,
  selectedKey,
  triggerLabel,
  triggerMuted,
}: SearchComboboxProps<T>) {
  const [open, setOpen] = React.useState(false);
  const [query, setQuery] = React.useState("");
  const [highlightedIndex, setHighlightedIndex] = React.useState(0);

  const filtered = React.useMemo(() => {
    if (!query) return items;
    const q = query.toLowerCase();
    return items.filter((item) => matches(item, q));
  }, [items, matches, query]);

  function handleOpenChange(next: boolean) {
    setOpen(next);
    if (!next) {
      setQuery("");
      setHighlightedIndex(0);
    }
  }

  function selectItem(item: T) {
    onSelect(item);
    handleOpenChange(false);
  }

  function handleKeyDown(e: React.KeyboardEvent) {
    if (filtered.length === 0) return;

    switch (e.key) {
      case "ArrowDown": {
        e.preventDefault();
        setHighlightedIndex((i) => (i + 1) % filtered.length);
        break;
      }
      case "ArrowUp": {
        e.preventDefault();
        setHighlightedIndex((i) => (i - 1 + filtered.length) % filtered.length);
        break;
      }
      case "Enter": {
        e.preventDefault();
        const target = filtered[highlightedIndex];
        if (target) selectItem(target);
        break;
      }
      case "Escape": {
        e.preventDefault();
        handleOpenChange(false);
        break;
      }
    }
  }

  return (
    <Popover onOpenChange={handleOpenChange} open={open}>
      <PopoverTrigger asChild>
        <button
          aria-expanded={open}
          className={cn(
            "flex h-9 w-full items-center justify-between rounded-md border border-input bg-transparent px-3 text-sm shadow-xs transition-colors focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50",
            triggerMuted && "text-muted-foreground",
          )}
          disabled={disabled}
          id={id}
          role="combobox"
          type="button"
        >
          <span className="truncate">{triggerLabel}</span>
          <ChevronsUpDown className="ml-2 h-4 w-4 shrink-0 text-muted-foreground" />
        </button>
      </PopoverTrigger>
      <PopoverContent
        align="start"
        className="w-(--radix-popover-trigger-width) p-0"
      >
        <div className="flex items-center gap-2 border-b border-border px-3 py-2">
          <Search className="h-4 w-4 shrink-0 text-muted-foreground" />
          <input
            autoCapitalize="none"
            autoComplete="off"
            autoCorrect="off"
            ref={(el) => el?.focus()}
            className="flex-1 bg-transparent text-sm outline-hidden placeholder:text-muted-foreground"
            onChange={(e) => {
              setQuery(e.target.value);
              setHighlightedIndex(0);
            }}
            onKeyDown={handleKeyDown}
            placeholder={searchPlaceholder}
            spellCheck={false}
            value={query}
          />
        </div>
        <div className="max-h-60 overflow-y-auto p-1">
          {loading ? (
            <p className="px-3 py-4 text-center text-xs text-muted-foreground">
              {loadingText ?? "Loading…"}
            </p>
          ) : filtered.length === 0 ? (
            <p className="px-3 py-4 text-center text-xs text-muted-foreground">
              {emptyText}
            </p>
          ) : (
            filtered.map((item, index) => (
              <button
                className={cn(
                  "flex w-full items-center gap-2 rounded-lg px-2 py-1.5 text-left text-sm transition-colors hover:bg-accent hover:text-accent-foreground",
                  getKey(item) === selectedKey && "bg-accent/50",
                  index === highlightedIndex &&
                    "bg-accent text-accent-foreground",
                )}
                key={getKey(item)}
                onClick={() => selectItem(item)}
                type="button"
              >
                <Check
                  className={cn(
                    "h-4 w-4 shrink-0",
                    getKey(item) === selectedKey ? "opacity-100" : "opacity-0",
                  )}
                />
                <span className="truncate">{renderItem(item)}</span>
              </button>
            ))
          )}
        </div>
      </PopoverContent>
    </Popover>
  );
}
