import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';

export function PauseMessageInput({ id, label, value, defaultValue, disabled, onSave }: {
  id: string; label: string; value: string; defaultValue: string; disabled: boolean; onSave: (text: string) => void;
}) {
  const {t}=useTranslation();
  const [draft,setDraft]=useState(value), [invalid,setInvalid]=useState(false);
  useEffect(()=>{setDraft(value);setInvalid(false);},[value]);
  const save=()=>{
    const valid=draft.trim().length>0 && draft.length<=2000;
    setInvalid(!valid);if(valid && draft!==value)onSave(draft);
  };
  return <div className="space-y-1">
    <div className="flex items-center justify-between gap-2 text-xs">
      <label htmlFor={id} className="font-semibold">{label}</label>
      <button type="button" disabled={disabled} onMouseDown={event=>event.preventDefault()}
        onClick={()=>{setDraft(defaultValue);setInvalid(false);onSave(defaultValue);}}
        className="rounded border px-2 py-1 disabled:opacity-50">{t('pauseRestore')}</button>
    </div>
    <textarea id={id} rows={4} maxLength={2000} disabled={disabled} value={draft} aria-invalid={invalid}
      aria-describedby={`${id}-hint${invalid?` ${id}-error`:''}`} onBlur={save} onChange={event=>{setDraft(event.target.value);setInvalid(false);}}
      className="w-full min-w-0 resize-y select-text rounded border border-slate-300 dark:border-slate-600 bg-white dark:bg-slate-800 p-2 text-xs leading-relaxed disabled:opacity-50" />
    <p id={`${id}-hint`} className="text-[11px] text-slate-500 dark:text-slate-400">{t('pauseMessageHint')}</p>
    {invalid && <p id={`${id}-error`} role="alert" className="text-xs text-rose-600">{t('pauseMessageInvalid')}</p>}
  </div>;
}
