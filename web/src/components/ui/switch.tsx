import { Switch as SwitchPrimitive } from "@base-ui/react/switch";

import { cn } from "~/lib/utils";

export function Switch({ className, ...props }: SwitchPrimitive.Root.Props) {
  return (
    <SwitchPrimitive.Root
      className={cn(
        "relative inline-flex h-[18px] w-[30px] shrink-0 items-center rounded-full p-[2px] outline-none transition-colors after:absolute after:-inset-2 after:content-[''] focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-1 focus-visible:ring-offset-background data-[checked]:bg-primary data-[unchecked]:bg-input data-[disabled]:opacity-50",
        className,
      )}
      {...props}
    >
      <SwitchPrimitive.Thumb className="pointer-events-none block size-[14px] rounded-full bg-white shadow-sm transition-transform duration-150 data-[checked]:translate-x-[12px]" />
    </SwitchPrimitive.Root>
  );
}
