import { ChevronDown } from "lucide-react";

import {
  setLinkPreviewStyle,
  useLinkPreviewStyle,
  type LinkPreviewStyle,
} from "@/shared/lib/linkPreviewStylePreference";
import { Button } from "@/shared/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { SettingsOptionGroup, SettingsOptionRow } from "./SettingsOptionGroup";

const LINK_PREVIEW_STYLE_OPTIONS: {
  value: LinkPreviewStyle;
  label: string;
  description: string;
}[] = [
  {
    value: "compact",
    label: "Compact",
    description: "Show links as compact horizontal cards",
  },
  {
    value: "rich",
    label: "Rich",
    description: "Unfurl links with larger images and descriptions",
  },
];

export function LinkPreviewStyleSetting() {
  const style = useLinkPreviewStyle();
  const activeOption =
    LINK_PREVIEW_STYLE_OPTIONS.find((option) => option.value === style) ??
    LINK_PREVIEW_STYLE_OPTIONS[0];

  return (
    <SettingsOptionGroup className="mt-8">
      <SettingsOptionRow>
        <div className="min-w-0">
          <p className="text-sm font-medium">Links</p>
          <p className="text-sm font-normal text-muted-foreground">
            {activeOption.description}
          </p>
        </div>
        <DropdownMenu modal={false}>
          <DropdownMenuTrigger asChild>
            <Button
              className="h-7 min-w-28 justify-between gap-1.5 rounded-full border border-border/50 bg-muted/45 px-2.5 text-xs font-medium text-foreground shadow-none hover:bg-muted/70"
              data-testid="link-preview-style-trigger"
              size="sm"
              type="button"
              variant="ghost"
            >
              <span className="truncate">{activeOption.label}</span>
              <ChevronDown className="h-4 w-4 text-muted-foreground" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" className="min-w-72">
            <DropdownMenuRadioGroup
              onValueChange={(next) =>
                setLinkPreviewStyle(next as LinkPreviewStyle)
              }
              value={style}
            >
              {LINK_PREVIEW_STYLE_OPTIONS.map((option) => (
                <DropdownMenuRadioItem
                  data-testid={`link-preview-style-${option.value}`}
                  key={option.value}
                  value={option.value}
                >
                  <span className="flex min-w-0 flex-col">
                    <span className="font-medium">{option.label}</span>
                    <span className="text-2xs text-muted-foreground">
                      {option.description}
                    </span>
                  </span>
                </DropdownMenuRadioItem>
              ))}
            </DropdownMenuRadioGroup>
          </DropdownMenuContent>
        </DropdownMenu>
      </SettingsOptionRow>
    </SettingsOptionGroup>
  );
}
