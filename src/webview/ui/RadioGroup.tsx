import { RadioGroup as Root } from '@base-ui/react/radio-group';
import { Radio as Base } from '@base-ui/react/radio';
import type { ComponentProps } from 'react';
import { cn, cnState } from './cn';
import { optionClass } from './DropdownMenu';

// Selection rows inside a multi-page popover; panels own their close actions.
function Item({ className, onKeyDownCapture, ...props }: ComponentProps<typeof Base.Root>) {
  return <Base.Root render={<button type="button" />} nativeButton {...props}
    onKeyDownCapture={event => {
      onKeyDownCapture?.(event);
      // These rows execute menu actions; preserve Enter as well as Radio's Space.
      if (event.key === 'Enter' && !event.defaultPrevented) {
        event.preventDefault();
        event.currentTarget.click();
      }
    }}
    className={cnState(cn(optionClass, 'data-[checked]:option-selected data-[checked]:hover:option-selected data-[checked]:focus-visible:option-selected'), className)} />;
}
export const RadioGroup = { Root, Item };
