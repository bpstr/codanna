import { lazy } from 'react';
const Calendar = lazy(() => import('./provider').then(module => ({ default: module.Calendar })));
export function FunctionShadowPicker() { function Calendar() { return null; } return <Calendar />; }
export function ClassShadowPicker() { class Calendar {} return <Calendar />; }
export function NestedLazyPicker() {
  const NestedCalendar = lazy(() => import('./provider').then(module => ({ default: module.Calendar })));
  return <NestedCalendar />;
}
