import { Menu as MenuPrimitive } from "@base-ui/react/menu";
import { CheckIcon } from "lucide-react";
import type * as React from "react";

import { cn } from "~/lib/utils";

export const Menu = MenuPrimitive.Root;
export const MenuTrigger = MenuPrimitive.Trigger;
export const MenuRadioGroup = MenuPrimitive.RadioGroup;

export const popupSurface =
  "rounded-lg border bg-popover text-popover-foreground shadow-[0_16px_40px_-18px_rgb(0_0_0/55%)] outline-none dark:shadow-[0_18px_44px_-18px_rgb(0_0_0/85%)] origin-[var(--transform-origin)] transition-[opacity,scale] duration-100 data-[ending-style]:scale-[0.98] data-[starting-style]:scale-[0.98] data-[ending-style]:opacity-0 data-[starting-style]:opacity-0";

export function MenuPopup({
  children,
  className,
  side = "bottom",
  align = "start",
  sideOffset = 4,
  ...props
}: MenuPrimitive.Popup.Props & {
  side?: MenuPrimitive.Positioner.Props["side"];
  align?: MenuPrimitive.Positioner.Props["align"];
  sideOffset?: number;
}) {
  return (
    <MenuPrimitive.Portal>
      <MenuPrimitive.Positioner
        side={side}
        align={align}
        sideOffset={sideOffset}
        className="z-[150]"
      >
        <MenuPrimitive.Popup
          className={cn(
            popupSurface,
            "min-w-44 max-w-[calc(100vw-2rem)]",
            className,
          )}
          {...props}
        >
          <div className="max-h-[var(--available-height)] overflow-y-auto p-1">
            {children}
          </div>
        </MenuPrimitive.Popup>
      </MenuPrimitive.Positioner>
    </MenuPrimitive.Portal>
  );
}

const itemClass =
  "flex min-h-7 cursor-pointer select-none items-center gap-2 rounded-md px-2 py-1 text-[13px] text-foreground outline-none data-[disabled]:pointer-events-none data-[highlighted]:bg-accent data-[disabled]:opacity-50 [&>svg]:size-4 [&>svg]:shrink-0 [&>svg:not([class*='text-'])]:text-muted-foreground";

export function MenuItem({
  className,
  destructive,
  ...props
}: MenuPrimitive.Item.Props & { destructive?: boolean }) {
  return (
    <MenuPrimitive.Item
      className={cn(
        itemClass,
        destructive && "text-destructive-foreground [&>svg]:text-current",
        className,
      )}
      {...props}
    />
  );
}

export function MenuRadioItem({
  className,
  children,
  ...props
}: MenuPrimitive.RadioItem.Props) {
  return (
    <MenuPrimitive.RadioItem
      closeOnClick
      className={cn(itemClass, "relative pe-7", className)}
      {...props}
    >
      {children}
      <MenuPrimitive.RadioItemIndicator className="absolute end-2">
        <CheckIcon className="size-3.5" />
      </MenuPrimitive.RadioItemIndicator>
    </MenuPrimitive.RadioItem>
  );
}

export function MenuLabel({
  className,
  ...props
}: React.ComponentProps<"div">) {
  return (
    <div
      className={cn(
        "px-2 pt-1.5 pb-1 font-medium text-[11px] text-muted-foreground",
        className,
      )}
      {...props}
    />
  );
}

export function MenuSeparator({
  className,
  ...props
}: MenuPrimitive.Separator.Props) {
  return (
    <MenuPrimitive.Separator
      className={cn("mx-1 my-1 h-px bg-border", className)}
      {...props}
    />
  );
}
