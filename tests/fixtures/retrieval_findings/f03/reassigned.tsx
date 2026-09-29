import { lazy } from 'react';
let ReassignedCalendar = lazy(() => import('./provider').then(module => ({ default: module.Calendar })));
ReassignedCalendar = () => null;
export function ReassignedPicker() { return <ReassignedCalendar />; }
