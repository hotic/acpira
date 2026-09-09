package com.github.hotic.acpira.platform

// One hit of the @ file search, as src/shared/protocol.ts FileHit: a file URI plus the path shown in the list
data class FileHit(val uri: String, val path: String)

// The ranking of src/host/fileRank.ts, kept identical so an @ search finds the same files in the same order in every IDE: subsequence
// match of the query against each path, best first. An empty query returns the list as given (callers pre-sort it shallow-first)
object FileRank {
    fun rank(files: List<FileHit>, query: String, limit: Int): List<FileHit> {
        val q = query.trim().lowercase()
        if (q.isEmpty()) return files.take(limit)
        return files.mapNotNull { f -> score(f.path.lowercase(), q)?.let { f to it } }
            .sortedByDescending { it.second }
            .take(limit)
            .map { it.first }
    }

    // null when the query is not a subsequence of the path. Consecutive characters, characters at word starts, and characters inside the
    // file name score higher; among equals, shorter paths win (the length penalty is small enough never to flip a real score difference)
    private fun score(path: String, q: String): Double? {
        val base = path.lastIndexOf('/') + 1
        var from = 0
        var prev = -2
        var s = 0
        for (ch in q) {
            val i = path.indexOf(ch, from)
            if (i < 0) return null
            s += 1 + (if (i == prev + 1) 2 else 0) + (if (i >= base) 2 else 0) + (if (i == 0 || path[i - 1] in "/._-") 3 else 0)
            prev = i
            from = i + 1
        }
        return s - minOf(path.length, 500) / 1000.0
    }
}
