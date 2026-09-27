import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { TagInput } from './TagInput';
import type { Category } from '../../types';

const CATS: Category[] = [{ id: 'c1', name: 'Python', color: '#0f0' }];

afterEach(cleanup);

describe('TagInput inline creation', () => {
  it('hides the creation input without onCreate', () => {
    render(<TagInput selected={[]} categories={CATS} onChange={() => {}} />);
    expect(screen.queryByLabelText('New category…')).toBeNull();
  });

  it('calls onCreate with the trimmed new name on Enter', async () => {
    const onCreate = vi.fn().mockResolvedValue(undefined);
    const onChange = vi.fn();
    render(<TagInput selected={[]} categories={[]} onChange={onChange} onCreate={onCreate} />);
    const input = screen.getByLabelText('New category…') as HTMLInputElement;
    fireEvent.change(input, { target: { value: '  Rust  ' } });
    fireEvent.keyDown(input, { key: 'Enter' });
    await waitFor(() => expect(onCreate).toHaveBeenCalledWith('Rust'));
    await waitFor(() => expect(input.value).toBe(''));
    expect(onChange).not.toHaveBeenCalled();
  });

  it('selects an existing category instead of creating a duplicate', () => {
    const onCreate = vi.fn();
    const onChange = vi.fn();
    render(<TagInput selected={[]} categories={CATS} onChange={onChange} onCreate={onCreate} />);
    fireEvent.change(screen.getByLabelText('New category…'), { target: { value: 'python' } });
    fireEvent.click(screen.getByText('+ CATEGORY'));
    expect(onCreate).not.toHaveBeenCalled();
    expect(onChange).toHaveBeenCalledWith(['Python']);
  });

  it('keeps the draft when creation fails', async () => {
    const onCreate = vi.fn().mockRejectedValue(new Error('boom'));
    render(<TagInput selected={[]} categories={[]} onChange={() => {}} onCreate={onCreate} />);
    const input = screen.getByLabelText('New category…') as HTMLInputElement;
    fireEvent.change(input, { target: { value: 'Go' } });
    fireEvent.click(screen.getByText('+ CATEGORY'));
    await waitFor(() => expect(onCreate).toHaveBeenCalled());
    expect(input.value).toBe('Go');
  });
});
