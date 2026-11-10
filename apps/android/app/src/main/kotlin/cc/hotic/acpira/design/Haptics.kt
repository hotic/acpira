package cc.hotic.acpira.design

import androidx.compose.ui.hapticfeedback.HapticFeedback
import androidx.compose.ui.hapticfeedback.HapticFeedbackType

// The app's haptic vocabulary (apps/ios AcpiraDesign/Haptics.swift): views name the moment, never a feedback type, so
// what each moment feels like is decided here, in one table. Sparse on purpose: nothing plays on streamed output, plain
// navigation, or where the platform already plays (system switches, the keyboard). A long press that opens a menu or
// lifts a row plays `HapticFeedbackType.LongPress` at its own call site, as the system's long press does. No in-app
// switch: the system's touch feedback setting governs. Compose maps each type to the nearest constant the device has.

/** A moment that plays haptic feedback, and the feedback it plays */
enum class Haptic(val type: HapticFeedbackType) {
  /** A prompt, or a question's answers, leave the phone */
  Send(HapticFeedbackType.Confirm),
  /** The running turn, or one of its background tasks, is told to stop */
  Stop(HapticFeedbackType.VirtualKey),
  /** A permission or a plan is allowed */
  Approve(HapticFeedbackType.Confirm),
  /** A permission is rejected */
  Reject(HapticFeedbackType.Reject),
  /** A value changes in one of the app's own pickers (the composer's beans, the agent ring, a question's options) */
  Pick(HapticFeedbackType.SegmentTick),
  /** A drag passes a detent: a swiped row reaches its stop ([Detent]) */
  Detent(HapticFeedbackType.GestureThresholdActivate),
  /** A dragged row lands on its drop target */
  Drop(HapticFeedbackType.GestureEnd),
  /** A button put text on the clipboard */
  Copy(HapticFeedbackType.ContextClick),
  /** The turn on screen finished */
  Finished(HapticFeedbackType.Confirm),
  /** The turn on screen ended in an error, or a pairing was refused */
  Failed(HapticFeedbackType.Reject),
  /** A computer paired */
  Paired(HapticFeedbackType.Confirm),
}

fun HapticFeedback.play(haptic: Haptic) = performHapticFeedback(haptic.type)

/**
 * A detent a drag can pass: [cross] says when to play [Haptic.Detent], once per crossing outwards, never for a drag
 * that starts already past it nor while the finger stays past it. [past]: where the drag starts
 */
class Detent(private var past: Boolean = false) {
  /** Moves to [now] (past the detent or not); true when this move crossed it outwards */
  fun cross(now: Boolean): Boolean {
    val crossed = now && !past
    past = now
    return crossed
  }
}
