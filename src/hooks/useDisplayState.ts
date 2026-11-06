import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { DisplayState } from '../types';

export function mergeDisplayState(current:DisplayState|null,next:DisplayState):DisplayState|null {
  if (!next || !Number.isSafeInteger(next.revision) || next.revision<0
    || !['codex','claude','dsh'].includes(next.effectiveProvider)
    || typeof next.sourceKey!=='string' || !Number.isSafeInteger(next.sourceGeneration)
    || !Array.isArray(next.candidates) || !Array.isArray(next.pauseReasons)) return current;
  return current&&current.revision>=next.revision?current:next;
}
export function useDisplayState() {
  const [display,setDisplay]=useState<DisplayState|null>(null);
  useEffect(()=>{
    let active=true;let unlisten:(()=>void)|undefined;
    const apply=(next:DisplayState)=>{if(active)setDisplay(current=>mergeDisplayState(current,next));};
    void listen<DisplayState>('display_updated',event=>apply(event.payload)).then(async fn=>{
      if(!active){fn();return;}unlisten=fn;
      try{apply(await invoke<DisplayState>('get_display_state'));}catch{/* Startup may still be restoring the app. */}
    }).catch(()=>{});
    return()=>{active=false;unlisten?.();};
  },[]);
  return display;
}
