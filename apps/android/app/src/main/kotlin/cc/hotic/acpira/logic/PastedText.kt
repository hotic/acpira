package cc.hotic.acpira.logic

// Ported from apps/ios Chat/PastedText.swift (shared/pastedText.ts): a long paste waits in the composer as a text
// attachment instead of flooding the field. Only an insertion of pasted size is classified, never typing, dictation or
// a restored draft.

object PastedText {
  const val CHARACTERS = 2000
  const val LINES = 20
  const val MAX_BYTES = 1024 * 1024

  sealed interface Result {
    data object Inline : Result
    /** Goes as a text draft carrying the pasted text verbatim */
    data object Attachment : Result
    data object TooBig : Result
  }

  /** Counted as JavaScript does: UTF-16 units, and CRLF, a lone CR and LF each one line break */
  fun collect(text: String): Result {
    if (text.length < CHARACTERS) {
      var count = 1
      var previous = ' '
      for (c in text) {
        if (c == '\r' || (c == '\n' && previous != '\r')) count++
        if (count >= LINES) break
        previous = c
      }
      if (count < LINES) return Result.Inline
    }
    return if (text.toByteArray(Charsets.UTF_8).size <= MAX_BYTES) Result.Attachment else Result.TooBig
  }
}
