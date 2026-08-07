import { Check, ChevronsUpDown } from "lucide-react";
import * as React from "react";

import { cn } from "@/shared/lib/cn";
import { Input } from "@/shared/ui/input";
import { Popover, PopoverContent, PopoverTrigger } from "@/shared/ui/popover";

type SearchSelectProps<TItem> = {
  disabled?: boolean;
  emptyLabel?: string;
  getLabel: (item: TItem) => string;
  getValue: (item: TItem) => string;
  items: TItem[];
  loading?: boolean;
  loadingLabel?: string;
  onChange: (value: string) => void;
  placeholder?: string;
  searchPlaceholder?: string;
  testId?: string;
  value: string;
};

/**
 * A searchable dropdown: a trigger button that opens a popover containing a
 * text filter input and a clickable, filtered list. Use this instead of a
 * native `<select>` when the option list can grow large enough that
 * type-to-filter meaningfully helps (100+ items).
 */
export function SearchSelect<TItem>({
  disabled,
  emptyLabel = "No matches",
  getLabel,
  getValue,
  items,
  loading,
  loadingLabel = "Loading...",
  onChange,
  placeholder = "Select...",
  searchPlaceholder = "Search...",
  testId,
  value,
}: SearchSelectProps<TItem>) {
  const [open, setOpen] = React.useState(false);
  const [query, setQuery] = React.useState("");
  const inputRef = React.useRef<HTMLInputElement>(null);

  const selectedItem = React.useMemo(
    () => items.find((item) => getValue(item) === value),
    [items, getValue, value],
  );

  const filteredItems = React.useMemo(() => {
    const trimmedQuery = query.trim().toLowerCase();
    if (!trimmedQuery) return items;
    return items.filter((item) =>
      getLabel(item).toLowerCase().includes(trimmedQuery),
    );
  }, [items, getLabel, query]);

  return (
    <Popover
      onOpenChange={(nextOpen) => {
        setOpen(nextOpen);
        if (nextOpen) {
          setQuery("");
          globalThis.setTimeout(() => inputRef.current?.focus(), 0);
        }
      }}
      open={open}
    >
      <PopoverTrigger asChild>
        <button
          className={cn(
            "flex h-8 w-full items-center justify-between gap-2 bg-transparent px-0 py-0 text-left text-sm text-muted-foreground/55 outline-none transition-colors duration-150 ease-out disabled:cursor-not-allowed disabled:opacity-50",
            selectedItem && "text-foreground",
          )}
          data-testid={testId}
          disabled={disabled}
          type="button"
        >
          <span className="truncate">
            {loading
              ? loadingLabel
              : selectedItem
                ? getLabel(selectedItem)
                : placeholder}
          </span>
          <ChevronsUpDown className="h-3.5 w-3.5 shrink-0 opacity-50" />
        </button>
      </PopoverTrigger>
      <PopoverContent
        align="start"
        className="w-(--radix-popover-trigger-width) p-0"
      >
        <div className="border-b border-border/60 p-1.5">
          <Input
            className="h-8"
            data-testid={testId ? `${testId}-search` : undefined}
            onChange={(event) => setQuery(event.target.value)}
            placeholder={searchPlaceholder}
            ref={inputRef}
            value={query}
          />
        </div>
        <div className="max-h-60 overflow-y-auto p-1">
          {filteredItems.length === 0 ? (
            <p className="px-2 py-3 text-center text-sm text-muted-foreground">
              {emptyLabel}
            </p>
          ) : (
            filteredItems.map((item) => {
              const itemValue = getValue(item);
              const isSelected = itemValue === value;
              return (
                <button
                  className={cn(
                    "flex min-h-9 w-full cursor-default select-none items-center gap-2 rounded-lg py-2 pl-2 pr-4 text-left text-sm outline-hidden transition-colors hover:bg-muted/50 focus:bg-muted/50 focus:text-foreground",
                  )}
                  data-testid={testId ? `${testId}-option` : undefined}
                  key={itemValue}
                  onClick={() => {
                    onChange(itemValue);
                    setOpen(false);
                  }}
                  type="button"
                >
                  <Check
                    className={cn(
                      "h-3.5 w-3.5 shrink-0",
                      isSelected ? "opacity-100" : "opacity-0",
                    )}
                  />
                  <span className="truncate">{getLabel(item)}</span>
                </button>
              );
            })
          )}
        </div>
      </PopoverContent>
    </Popover>
  );
}
