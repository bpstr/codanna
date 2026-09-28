import { lazy } from 'react';
const BlockCalendar = lazy(() => import('./provider').then(module => { module = { Calendar: () => null }; return { default: module.Calendar }; }));
export function BlockPicker() { return <BlockCalendar />; }
