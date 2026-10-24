import * as vscode from 'vscode';
import type { FileHit } from '@shared/protocol';
import { rankFiles } from './fileRank';

// How long one findFiles listing is reused before the next search lists again
const TTL = 15_000;
const MAX_FILES = 20_000;
const EXCLUDE = '{**/node_modules/**,**/.git/**}';

// Workspace file index behind the composer's @ mention: listed with findFiles and cached briefly, ranked here so the webview only ever gets the top hits
export class WorkspaceFiles {
  private cache?: { at: number; files: FileHit[] };
  private listing?: Promise<FileHit[]>;

  async search(query: string, limit = 20): Promise<FileHit[]> {
    return rankFiles(await this.list(), query, limit);
  }

  private list(): Promise<FileHit[]> {
    if (this.cache && Date.now() - this.cache.at < TTL) return Promise.resolve(this.cache.files);
    // Concurrent searches while a listing is in flight share it
    this.listing ??= this.refresh().finally(() => { this.listing = undefined; });
    return this.listing;
  }

  private async refresh(): Promise<FileHit[]> {
    const uris = await vscode.workspace.findFiles('**/*', EXCLUDE, MAX_FILES);
    const files = uris
      .map(u => ({ uri: u.toString(), path: vscode.workspace.asRelativePath(u, false) }))
      .sort((a, b) => depth(a.path) - depth(b.path) || a.path.localeCompare(b.path));
    this.cache = { at: Date.now(), files };
    return files;
  }
}

function depth(path: string): number {
  return path.split('/').length;
}
