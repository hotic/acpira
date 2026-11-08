package cc.hotic.acpira.store

import cc.hotic.acpira.logic.FileLink
import cc.hotic.acpira.logic.FileLinks
import cc.hotic.acpira.logic.code
import cc.hotic.acpira.model.DiffLine
import cc.hotic.acpira.model.ProjectHunk
import org.junit.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull
import kotlin.test.assertTrue

/** The file page's rows as FileViewer.tsx viewRows lays them out, its path helpers and the app's own file links (apps/ios FilesTests.swift) */
class FilesTest {
  @Test fun removedLinesStandBeforeTheirReplacement() {
    // Line 2 replaced by two lines, line 4 removed at the end
    val rows = fileLines("a\nB1\nB2\nc\n", listOf(ProjectHunk(2, 2, listOf("b")), ProjectHunk(5, 0, listOf("d"))))
    assertEquals(listOf(DiffLine.Kind.ctx, DiffLine.Kind.del, DiffLine.Kind.add, DiffLine.Kind.add, DiffLine.Kind.ctx, DiffLine.Kind.del), rows.map { it.kind })
    assertEquals(listOf("a", "b", "B1", "B2", "c", "d"), rows.map { it.code })
    assertEquals(listOf(1, null, 2, 3, 4, null), rows.map { it.newLine })
    assertEquals(listOf(1, 2, null, null, 3, 4), rows.map { it.oldLine })
    assertEquals(listOf(1, 5), changeStarts(rows))
  }

  @Test fun crlfAndNoHunks() {
    val rows = fileLines("x\r\ny", emptyList())
    assertEquals(listOf("x", "y"), rows.map { it.code })
    assertTrue(changeStarts(rows).isEmpty())
  }

  @Test fun pathParts() {
    assertEquals("App.tsx", fileName("src/ui/App.tsx"))
    assertEquals("src/ui", folderOf("src/ui/App.tsx"))
    assertEquals("", folderOf("README.md"))
  }

  @Test fun markdownPreview() {
    assertTrue(isMarkdownPath("docs/A.MD") && isMarkdownPath("x.mdx") && !isMarkdownPath("md") && !isMarkdownPath("a.ts"))
    // Front matter turns into a YAML block; a rule later in the file stays
    assertEquals("```yaml\nname: x\n```\n\n# T\n---\n", markdownPreviewText("---\nname: x\n---\n# T\n---\n"))
    assertEquals("# T\n---\nname: x\n---\n", markdownPreviewText("# T\n---\nname: x\n---\n"))
    assertEquals("docs/a.md", joinPath("docs/dev", "../a.md"))
    assertEquals("/w/docs/b/c.md", joinPath("/w/docs", "./b/c.md"))
    assertNull(joinPath("", "../a.md"))
    assertNull(joinPath("docs", "/abs/a.md"))
  }

  @Test fun ownLinkRoundTrips() {
    val link = FileLink("/repo/my file#1.ts", 40)
    val url = FileLinks.url(link)
    assertTrue(url.startsWith("${FileLinks.SCHEME}:"))
    assertEquals(link, FileLinks.decode(url))
    assertEquals(link, FileLinks.parse(url))
    assertNull(FileLinks.decode("https://example.com/a.ts"))
  }
}
