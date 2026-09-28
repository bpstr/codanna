import { lazy } from 'react';
const DuplicateCalendar = lazy(() => import('./provider').then(module => ({ default: module.Calendar, default: module.Calendar })));
const SpreadCalendar = lazy(() => import('./provider').then(module => ({ ...module, default: module.Calendar })));
export function DuplicatePicker() { return <DuplicateCalendar />; }
export function SpreadPicker() { return <SpreadCalendar />; }
