import { Select as SelectPrimitive } from "@base-ui/react/select";
import { CheckIcon, ChevronDownIcon } from "lucide-react";
import type * as React from "react";
import { useId } from "react";

import { cn } from "~/lib/utils";
import { popupSurface } from "./menu";

export interface SelectOption<T extends string = string> {
  value: T;
  label: React.ReactNode;
  description?: React.ReactNode;
  group?: string;
  disabled?: boolean;
}

/** A compact select. Options may be grouped; the trigger shows the chosen label. */
export function Select<T extends string>({
  value,
  onChange,
  options,
  className,
  size = "default",
  variant = "outline",
  disabled,
  placeholder,
  "aria-label": ariaLabel,
  popupClassName,
}: {
  value: T;
  onChange: (value: T) => void;
  options: SelectOption<T>[];
  className?: string;
  size?: "default" | "sm" | "xs";
  variant?: "outline" | "ghost";
  disabled?: boolean;
  placeholder?: string;
  "aria-label"?: string;
  popupClassName?: string;
}) {
  const groups = [...new Set(options.map((option) => option.group ?? ""))];
  const selected = options.find((option) => option.value === value);
  const labelId = useId();
  const valueId = useId();
  return (
    <SelectPrimitive.Root
      value={value}
      onValueChange={(next) => {
        if (next !== null) onChange(next as T);
      }}
      disabled={disabled}
    >
      <SelectPrimitive.Trigger
        aria-labelledby={ariaLabel ? `${labelId} ${valueId}` : valueId}
        className={cn(
          "inline-flex min-w-0 select-none items-center justify-between gap-1.5 rounded-[var(--control-radius)] border text-left text-sm outline-none transition-[color,background-color,border-color,box-shadow,scale] focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/20 active:scale-[0.98] data-[disabled]:opacity-50",
          variant === "outline"
            ? "border-input bg-background shadow-xs/5 hover:bg-accent/50 dark:bg-input/30"
            : "border-transparent text-muted-foreground hover:bg-accent hover:text-foreground data-[popup-open]:bg-accent data-[popup-open]:text-foreground",
          size === "default" && "h-10 px-3",
          size === "sm" && "h-9 px-2.5 text-ui",
          size === "xs" && "h-8 rounded-md px-2 text-xs",
          className,
        )}
      >
        {ariaLabel && (
          <span id={labelId} className="sr-only">
            {ariaLabel}
          </span>
        )}
        <span id={valueId} className="min-w-0 flex-1 truncate">
          {selected?.label ?? (
            <span className="text-muted-foreground">{placeholder}</span>
          )}
        </span>
        <SelectPrimitive.Icon>
          <ChevronDownIcon className="size-3.5 opacity-60" />
        </SelectPrimitive.Icon>
      </SelectPrimitive.Trigger>
      <SelectPrimitive.Portal>
        <SelectPrimitive.Positioner
          sideOffset={4}
          alignItemWithTrigger={false}
          className="z-[160]"
        >
          <SelectPrimitive.Popup
            className={cn(
              popupSurface,
              "min-w-[var(--anchor-width)]",
              popupClassName,
            )}
          >
            <SelectPrimitive.List className="max-h-[min(24rem,var(--available-height))] overflow-y-auto p-1">
              {groups.map((group) => (
                <SelectPrimitive.Group key={group || "default"}>
                  {group && (
                    <SelectPrimitive.GroupLabel className="px-2 pt-1.5 pb-1 font-medium text-2xs text-muted-foreground">
                      {group}
                    </SelectPrimitive.GroupLabel>
                  )}
                  {options
                    .filter((option) => (option.group ?? "") === group)
                    .map((option) => (
                      <SelectPrimitive.Item
                        key={option.value}
                        value={option.value}
                        disabled={option.disabled}
                        className="relative flex min-h-9 cursor-pointer select-none items-center gap-2 rounded-sm py-1.5 ps-2 pe-7 text-ui outline-none data-[disabled]:opacity-50 data-[highlighted]:bg-accent"
                      >
                        <div className="min-w-0">
                          <SelectPrimitive.ItemText className="block truncate">
                            {option.label}
                          </SelectPrimitive.ItemText>
                          {option.description && (
                            <div className="truncate text-2xs text-muted-foreground">
                              {option.description}
                            </div>
                          )}
                        </div>
                        <SelectPrimitive.ItemIndicator className="absolute end-2">
                          <CheckIcon className="size-3.5" />
                        </SelectPrimitive.ItemIndicator>
                      </SelectPrimitive.Item>
                    ))}
                </SelectPrimitive.Group>
              ))}
            </SelectPrimitive.List>
          </SelectPrimitive.Popup>
        </SelectPrimitive.Positioner>
      </SelectPrimitive.Portal>
    </SelectPrimitive.Root>
  );
}
