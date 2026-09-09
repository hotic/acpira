package com.github.hotic.acpira.platform

import org.junit.Assert.assertEquals
import org.junit.Test

// The cases of test/fileRank.test.ts, so the two scorers stay in step
class FileRankTest {
    private fun hit(path: String) = FileHit("file:///repo/$path", path)
    private fun paths(hits: List<FileHit>) = hits.map { it.path }
    private val files = listOf("README.md", "src/cli/args.ts", "src/cli/index.ts", "src/foo.ts", "test/args.test.ts", "docs/architecture.md").map(::hit)

    @Test fun `empty query returns the list as given, capped`() {
        assertEquals(listOf("README.md", "src/cli/args.ts", "src/cli/index.ts"), paths(FileRank.rank(files, "", 3)))
    }

    @Test fun `subsequence match, file-name hits and word starts outrank scattered matches, non-matches are dropped`() {
        assertEquals(listOf("src/cli/args.ts", "test/args.test.ts"), paths(FileRank.rank(files, "args", 10)))
        assertEquals("src/cli/args.ts", paths(FileRank.rank(files, "ats", 10))[0])
        assertEquals(emptyList<FileHit>(), FileRank.rank(files, "zzz", 10))
    }

    @Test fun `a long path that matches is never filtered out by the length tie-breaker`() {
        val long = hit("parent/${"x".repeat(150)}/file.ts")
        assertEquals(listOf(long.path), paths(FileRank.rank(listOf(long), "a", 10)))
        assertEquals(listOf(long.path), paths(FileRank.rank(listOf(long), "f", 10)))
    }

    @Test fun `shorter path wins a tie`() {
        val a = hit("src/a/b/c/util.ts")
        val b = hit("src/util.ts")
        assertEquals(listOf(b.path, a.path), paths(FileRank.rank(listOf(a, b), "util", 10)))
    }
}
