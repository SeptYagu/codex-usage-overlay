import { cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { PauseMessageInput } from './PauseMessageInput';
import '../i18n';
afterEach(cleanup);
it('saves only on blur, rejects blank drafts and restores only this message',()=>{
  const save=vi.fn();const view=render(<PauseMessageInput id="test-message" label="Message" value="saved" defaultValue="Pause" disabled={false} onSave={save}/>);
  const input=view.getByRole('textbox');fireEvent.change(input,{target:{value:'edited\nmessage'}});expect(save).not.toHaveBeenCalled();fireEvent.blur(input);expect(save).toHaveBeenLastCalledWith('edited\nmessage');
  save.mockClear();fireEvent.change(input,{target:{value:' \n '}});fireEvent.blur(input);expect(save).not.toHaveBeenCalled();expect(view.getByRole('alert')).toBeTruthy();
  fireEvent.click(view.getByRole('button'));expect(save).toHaveBeenCalledWith('Pause');expect(view.queryByRole('alert')).toBeNull();
});
it('uses the same UTF-16 boundary as the native validator',()=>{
  const save=vi.fn();const view=render(<PauseMessageInput id="unicode-message" label="Unicode" value="saved" defaultValue="Pause" disabled={false} onSave={save}/>);
  const input=view.getByRole('textbox');fireEvent.change(input,{target:{value:'😀'.repeat(1000)}});fireEvent.blur(input);expect(save).toHaveBeenCalledTimes(1);
  fireEvent.change(input,{target:{value:'😀'.repeat(1001)}});fireEvent.blur(input);expect(save).toHaveBeenCalledTimes(1);expect(view.getByRole('alert')).toBeTruthy();
});
