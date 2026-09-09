package com.github.hotic.acpira.platform

import com.intellij.openapi.application.ReadAction
import com.intellij.openapi.fileTypes.FileTypeManager
import com.intellij.openapi.project.Project
import com.intellij.openapi.roots.ContentIterator
import com.intellij.openapi.roots.ProjectFileIndex
import com.intellij.openapi.vfs.VfsUtilCore
import com.intellij.openapi.vfs.VirtualFile
import com.intellij.openapi.vfs.VirtualFileFilter
import java.nio.file.Path
import java.nio.file.Paths
import java.util.concurrent.CompletableFuture

// The @ mention index behind searchFiles: the project's content as the IDE knows it (content roots minus excluded folders, ignored
// names and the usual dependency directories), listed with the VFS instead of walking the disk, capped and cached like the VS Code
// WorkspaceFiles, ranked with the same scorer. Called from the sidecar's reader thread; the listing runs in a read action
class IdeFiles(private val project: Project) {
    private var cache: Pair<Long, List<FileHit>>? = null
    private var listing: CompletableFuture<List<FileHit>>? = null

    fun search(query: String, limit: Int = 20): List<FileHit> = FileRank.rank(list(), query, limit)

    private fun list(): List<FileHit> {
        val now = System.currentTimeMillis()
        synchronized(this) {
            cache?.let { (at, files) -> if (now - at < TTL_MS) return files }
            // Concurrent searches while a listing is in flight share it
            listing?.let { return it.join() }
            val f = CompletableFuture<List<FileHit>>()
            listing = f
            try {
                val files = ReadAction.nonBlocking<List<FileHit>> { walk() }.expireWith(project).executeSynchronously()
                cache = now to files
                f.complete(files)
                return files
            } catch (e: Throwable) {
                f.completeExceptionally(e)
                throw e
            } finally { listing = null }
        }
    }

    private fun walk(): List<FileHit> {
        val base = project.basePath?.let { Paths.get(it) }
        val index = ProjectFileIndex.getInstance(project)
        val types = FileTypeManager.getInstance()
        val files = ArrayList<FileHit>()
        val each = ContentIterator { vf ->
            if (!vf.isDirectory && !types.isFileIgnored(vf)) {
                files += FileHit(Paths.get(vf.path).toUri().toString(), relativePath(vf, base, index))
            }
            files.size < MAX_FILES
        }
        // A directory the filter refuses is not descended into
        val skip = VirtualFileFilter { vf -> !vf.isDirectory || (vf.name !in EXCLUDE && !types.isFileIgnored(vf)) }
        index.iterateContent(each, skip)
        // Shallow files first, then by name, so an empty query and score ties come out the way the other hosts order them
        files.sortWith(compareBy<FileHit> { it.path.count { c -> c == '/' } }.thenBy { it.path })
        return files
    }

    // Shown relative to the project folder when the file is under it, to its content root otherwise (a project with several roots)
    private fun relativePath(vf: VirtualFile, base: Path?, index: ProjectFileIndex): String {
        val p = Paths.get(vf.path)
        if (base != null && p.startsWith(base)) return base.relativize(p).toString().replace('\\', '/')
        val root = index.getContentRootForFile(vf)
        return (if (root != null) VfsUtilCore.getRelativePath(vf, root) else null) ?: vf.path
    }

    companion object {
        private const val TTL_MS = 15_000L
        private const val MAX_FILES = 20_000
        private val EXCLUDE = setOf("node_modules", ".git")
    }
}
