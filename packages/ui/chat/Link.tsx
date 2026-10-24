import { isValidElement, useContext, type ComponentProps, type KeyboardEvent, type ReactNode } from 'react';
import type { ExtraProps } from 'streamdown';
import { vscodeApi } from '../vscodeApi';
import { cn } from '../ui/cn';
import { sessionIdOfLink } from '@shared/sessionLinks';
import { OpenSessionContext, OpenToolFileContext, decodeFileHref, decodeSessionHref, parseCodeSessionLink, parseFileLink, parseWrappedLink } from './fileLinks';

// Links in agent output: the webview cannot navigate, so clicks are forwarded to the host.
// Workspace paths go through openFile, session links switch to that session; http(s)/mailto stay on openExternal.
// A host is either VS Code (acquireVsCodeApi) or a shell that injected window.__acpiraApi (JCEF, LAB fixtures); without either
// (a bare LAB page) fall back to a plain window.open.
const hasApi = () => !!window.__acpiraApi || typeof acquireVsCodeApi === 'function';

function openExternal(url: string) {
  if (hasApi()) vscodeApi().postMessage({ type: 'openExternal', url });
  else window.open(url, '_blank', 'noopener');
}

function fileFromHref(href: string) {
  return decodeFileHref(href) ?? parseFileLink(href);
}

// ExtraProps: streamdown passes the hast node along; unused here but part of the component contract
export function Link({ href, children, node: _node, ...rest }: ComponentProps<'a'> & ExtraProps) {
  const openFile = useContext(OpenToolFileContext);
  const openSession = useContext(OpenSessionContext);
  return (
    <a
      href={href}
      onClick={e => {
        e.preventDefault();
        if (!href) return;
        const session = decodeSessionHref(href) ?? sessionIdOfLink(href);
        if (session) {
          openSession?.(session);
          return;
        }
        const file = fileFromHref(href);
        if (file) {
          openFile?.(file.path, file.line);
          return;
        }
        // Fragment links (GFM footnotes / back-references) stay in-page; only real URLs go external
        if (href.startsWith('#')) document.getElementById(href.slice(1))?.scrollIntoView();
        else openExternal(href);
      }}
      {...rest}
    >
      {children}
    </a>
  );
}

export function InlineFileCode({ children, className, node: _node, ...rest }: ComponentProps<'code'> & ExtraProps) {
  const openFile = useContext(OpenToolFileContext);
  const openSession = useContext(OpenSessionContext);
  const text = openFile || openSession ? textOf(children) : '';
  // A session link in backticks (bare or a whole `[title](link)`) opens the session, showing the label when there is one
  const session = openSession ? parseCodeSessionLink(text) : undefined;
  if (session) return <CodeLink className={className} title={session.label ? undefined : session.id} label={session.label ?? children} open={() => openSession!(session.id)} rest={rest} />;
  // Some models wrap the whole `[label](file:///…)` link in backticks; show the label, open the target.
  const wrapped = openFile ? parseWrappedLink(text) : undefined;
  const file = wrapped?.file ?? (openFile ? parseFileLink(text) : undefined);
  if (!openFile || !file) return <code className={className} {...rest}>{children}</code>;
  return <CodeLink className={className} title={file.path} label={wrapped ? wrapped.label : children} open={() => openFile(file.path, file.line)} rest={rest} />;
}

function CodeLink({ className, title, label, open, rest }: { className?: string; title?: string; label: ReactNode; open: () => void; rest: ComponentProps<'code'> }) {
  const onKeyDown = (e: KeyboardEvent<HTMLElement>) => {
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault();
      open();
    }
  };
  return (
    <code role="link" tabIndex={0} title={title} className={cn('cursor-pointer hover:underline focus-visible:underline', className)}
      onClick={open} onKeyDown={onKeyDown} {...rest}>{label}</code>
  );
}

function textOf(node: ReactNode): string {
  if (node == null || typeof node === 'boolean') return '';
  if (typeof node === 'string' || typeof node === 'number') return String(node);
  if (Array.isArray(node)) return node.map(textOf).join('');
  if (isValidElement(node)) return textOf((node.props as { children?: ReactNode }).children);
  return '';
}
