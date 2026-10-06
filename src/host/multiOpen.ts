// "Open in Editor Columns": N chat tabs side by side in equal-width editor groups. The pure parts live here (no vscode import),
// extension.ts drives the workbench commands with them

export const COLUMN_CHOICES = [2, 3, 4, 5, 6] as const;
export const DEFAULT_COLUMNS = 4;

// Workbench chrome closed before the columns are laid out, in this order. A fork (Cursor, Windsurf …) may lack one; a missing or
// failing command is skipped
export const CHROME_COMMANDS = ['workbench.action.closeSidebar', 'workbench.action.closeAuxiliaryBar', 'workbench.action.closePanel'] as const;

export interface ColumnPick {
  label: string;
  columns: number;
}

// Quick-pick items for 2…6 columns. The default is not marked `picked`: VS Code honours that flag only in a multi-select
// picker, so the caller makes `defaultColumnPick` the active item instead
export function columnPicks(): ColumnPick[] {
  return COLUMN_CHOICES.map(n => ({ label: String(n), columns: n }));
}

// The item a single-select picker starts on (highlighted, taken by Enter)
export function defaultColumnPick(picks: readonly ColumnPick[]): ColumnPick | undefined {
  return picks.find(p => p.columns === DEFAULT_COLUMNS);
}

// `vscode.setEditorLayout` argument: one horizontal row of n equal groups (orientation 0 = horizontal)
export function columnsLayout(n: number): { orientation: 0; groups: { size: number }[] } {
  return { orientation: 0, groups: Array.from({ length: n }, () => ({ size: 1 / n })) };
}

// The chrome commands this host actually registers, in order
export function availableChromeCommands(registered: readonly string[]): string[] {
  const known = new Set(registered);
  return CHROME_COMMANDS.filter(c => known.has(c));
}
