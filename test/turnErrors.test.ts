import { describe, expect, it } from 'vitest';
import { isContextLengthError } from '../src/shared/turnErrors';

describe('context overflow detection', () => {
  it.each([
    'The prompt to the model was too long. Try reducing the size of your context (including any rules, skills, etc.).',
    'Prompt is too long',
    'Context length exceeded',
    'This model has a maximum context length of 128000 tokens.',
  ])('recognizes the model input failure: %s', message => {
    expect(isContextLengthError({ message, kind: 'internal', retryable: true })).toBe(true);
  });

  it('accepts a structured context error without changing vendor metadata', () => {
    const error = { message: 'Bad request', kind: 'context_length_exceeded', retryable: true };
    expect(isContextLengthError(error)).toBe(true);
    expect(error.retryable).toBe(true);
  });

  it.each(['Tool output was too long', 'Maximum output tokens reached', 'Rate limit exceeded', 'Request took too long'])('keeps unrelated failures on their existing path: %s', message => {
    expect(isContextLengthError({ message })).toBe(false);
  });
});
