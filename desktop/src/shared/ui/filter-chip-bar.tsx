import { ChevronDown } from "lucide-react";

import { Button } from "@/shared/ui/button";
import { cn } from "@/shared/lib/cn";
import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";

/** A single multi-select filter dropdown, styled as a chip. Shows the
 * option count when one or more values are selected, otherwise the bare
 * label. Used to build basic client-side filter bars above flat lists
 * (project issues, pull requests). */
export function FilterChip<T extends string>({
  label,
  onChange,
  options,
  testId,
  value,
}: {
  label: string;
  onChange: (value: T[]) => void;
  options: Array<{ label: string; value: T }>;
  testId?: string;
  value: T[];
}) {
  const toggle = (option: T) => {
    onChange(
      value.includes(option)
        ? value.filter((item) => item !== option)
        : [...value, option],
    );
  };

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button
          aria-label={label}
          className={cn(
            "h-7 gap-1 rounded-full border px-2.5 text-xs font-medium",
            value.length > 0
              ? "border-primary/40 bg-primary/10 text-foreground"
              : "border-border/60 bg-background/60 text-muted-foreground",
          )}
          data-testid={testId}
          size="sm"
          variant="outline"
        >
          {label}
          {value.length > 0 ? (
            <span className="rounded-full bg-primary/20 px-1.5 text-2xs text-foreground">
              {value.length}
            </span>
          ) : null}
          <ChevronDown className="h-3.5 w-3.5" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" className="min-w-48">
        {options.map((option) => (
          <DropdownMenuCheckboxItem
            checked={value.includes(option.value)}
            key={option.value}
            onCheckedChange={() => toggle(option.value)}
            onSelect={(event) => event.preventDefault()}
          >
            {option.label}
          </DropdownMenuCheckboxItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

/** "Clear filters" chip — only render when a filter is active. */
export function ClearFiltersChip({
  onClear,
  testId,
}: {
  onClear: () => void;
  testId?: string;
}) {
  return (
    <Button
      className="h-7 rounded-full px-2.5 text-xs font-medium text-muted-foreground hover:text-foreground"
      data-testid={testId}
      onClick={onClear}
      size="sm"
      variant="ghost"
    >
      Clear filters
    </Button>
  );
}
