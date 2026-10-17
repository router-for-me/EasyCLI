import { useRef, useState } from 'react';
import { DndContext, PointerSensor, KeyboardSensor, closestCenter, useSensor, useSensors, type Modifier } from '@dnd-kit/core';
import { SortableContext, useSortable, sortableKeyboardCoordinates, rectSortingStrategy, arrayMove } from '@dnd-kit/sortable';
import { CSS } from '@dnd-kit/utilities';
import { pluginText } from '../i18n/plugins';
import { readPluginNavOrder, writePluginNavOrder, orderPluginNavEntries, mergePluginNavOrder } from '../services/pluginNavigation';
import { AppWindow, ChevronRight, Lock, GripVertical } from 'lucide-react';
import { useI18n } from '../i18n';
import type { PluginNavEntry } from '../services/pluginNavigation';

export const PLUGIN_CHILDREN_ID = 'nav-plugin-children';

export function PluginNavToggle({ expanded, onToggle }: { expanded: boolean; onToggle: () => void }) {
  const { t } = useI18n();
  return (
    <button
      type="button"
      className="nav-plugin-toggle"
      aria-expanded={expanded}
      aria-controls={PLUGIN_CHILDREN_ID}
      aria-label={t('app.nav.pluginPages')}
      onClick={onToggle}
    >
      <ChevronRight size={14} strokeWidth={1.75} aria-hidden="true" />
    </button>
  );
}

function SortablePluginEntry({ entry, active, locked, sortable, onSelect }: {
  entry: PluginNavEntry; active: string; locked: boolean; sortable: boolean; onSelect: (route: string) => void;
}) {
  const { t, locale } = useI18n();
  const { attributes, listeners, setNodeRef, setActivatorNodeRef, transform, transition, isDragging } = useSortable({ id: entry.route, disabled: locked || !sortable });
  const isActive = entry.route === active;
  return <div ref={setNodeRef} className={`nav-plugin-row${isDragging ? ' dragging' : ''}`} style={{ transform: CSS.Transform.toString(transform), transition }}>
    <button type="button" className={['nav-plugin-entry', isActive ? 'active' : '', locked ? 'locked' : ''].filter(Boolean).join(' ')}
      aria-current={isActive ? 'page' : undefined} data-plugin-route={entry.route} disabled={locked}
      title={locked ? t('app.nav.lockedHint') : entry.description} onClick={() => onSelect(entry.route)}>
      <AppWindow size={16} strokeWidth={1.75} aria-hidden="true" /><span>{entry.label}</span>
      {locked ? <Lock size={12} strokeWidth={1.75} className="nav-lock-icon" aria-hidden="true" /> : null}
    </button>
    {sortable && !locked && <button ref={setActivatorNodeRef} type="button" className="nav-plugin-drag" {...attributes} {...listeners}
      aria-label={`${pluginText('reorderPage', locale)}: ${entry.label}`} title={pluginText('reorderPage', locale)}>
      <GripVertical size={13} aria-hidden="true" />
    </button>}
  </div>;
}

export function PluginNavChildren({ entries, active, locked, onSelect }: {
  entries: PluginNavEntry[];
  active: string;
  locked: boolean;
  onSelect: (route: string) => void;
}) {
  const { t, locale } = useI18n();
  const [order, setOrder] = useState(readPluginNavOrder);
  const sorted = orderPluginNavEntries(entries, order);
  const group = useRef<HTMLDivElement>(null);
  const dragBounds = useRef<DOMRect | null>(null);
  const constrainDrag: Modifier = ({ transform, activeNodeRect }) => {
    const bounds = dragBounds.current;
    if (!bounds || !activeNodeRect) return transform;
    const clamp = (value: number, min: number, max: number) => Math.max(min, Math.min(max, value));
    return { ...transform,
      x: clamp(transform.x, bounds.left - activeNodeRect.left, bounds.right - activeNodeRect.right),
      y: clamp(transform.y, bounds.top - activeNodeRect.top, bounds.bottom - activeNodeRect.bottom),
    };
  };
  const sensors = useSensors(useSensor(PointerSensor, { activationConstraint: { distance: 5 } }), useSensor(KeyboardSensor, { coordinateGetter: sortableKeyboardCoordinates }));
  return <DndContext sensors={sensors} collisionDetection={closestCenter} modifiers={[constrainDrag]} autoScroll={false}
    onDragStart={() => {
      // Read layout positions before transforms; display:contents has no own bounds in compact mode.
      const rows = Array.from(group.current?.querySelectorAll<HTMLElement>('.nav-plugin-row') ?? []);
      const rects = rows.map(row => row.getBoundingClientRect());
      if (!rects.length) return;
      const left = Math.min(...rects.map(rect => rect.left));
      const top = Math.min(...rects.map(rect => rect.top));
      dragBounds.current = new DOMRect(left, top, Math.max(...rects.map(rect => rect.right)) - left, Math.max(...rects.map(rect => rect.bottom)) - top);
    }}
    accessibility={{ screenReaderInstructions: { draggable: pluginText('reorderHint', locale) } }}
    onDragEnd={({ active: dragged, over }) => {
      if (locked || !over || dragged.id === over.id) return;
      const from = sorted.findIndex(entry => entry.route === dragged.id);
      const to = sorted.findIndex(entry => entry.route === over.id);
      if (from < 0 || to < 0) return;
      const next = mergePluginNavOrder(order, arrayMove(sorted, from, to).map(entry => entry.route));
      setOrder(next); writePluginNavOrder(next);
    }}>
    <SortableContext items={sorted.map(entry => entry.route)} strategy={rectSortingStrategy}>
      <div ref={group} id={PLUGIN_CHILDREN_ID} className="nav-plugin-children" role="group" aria-label={t('app.nav.pluginPages')}>
        {sorted.map(entry => <SortablePluginEntry key={entry.route} entry={entry} active={active} locked={locked} sortable={sorted.length > 1} onSelect={onSelect} />)}
      </div>
    </SortableContext>
  </DndContext>;
}
