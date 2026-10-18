import { RadioGroup as Root } from '@base-ui/react/radio-group';
import { Radio as Base } from '@base-ui/react/radio';
import type { ComponentProps } from 'react';
import { cn, cnState } from './cn';
import { optionClass } from './DropdownMenu';

// Selection rows inside a multi-page popover; panels own their close actions.
// Radio sets no data-highlighted, so the pointer and keyboard fills come from :hover / :focus-visible. The checked row has
// no fill of its own: its check mark marks it, as in DropdownMenu (one fill per list)
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
    className={cnState(cn(optionClass, 'transition-colors enabled:hover:option-highlighted focus-visible:option-highlighted'), className)} />;
}
export const RadioGroup = { Root, Item };
