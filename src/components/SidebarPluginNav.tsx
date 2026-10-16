import {
  Fragment,
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
  type MouseEvent as ReactMouseEvent,
  type ReactNode,
} from 'react';
import { createPortal } from 'react-dom';
import {
  DndContext,
  DragOverlay,
  KeyboardSensor,
  MeasuringStrategy,
  PointerSensor,
  closestCenter,
  useDraggable,
  useDroppable,
  useSensor,
  useSensors,
  type Active,
  type Announcements,
  type DragEndEvent,
  type DragMoveEvent,
  type DragStartEvent,
  type DraggableAttributes,
  type KeyboardCoordinateGetter,
  type Over,
} from '@dnd-kit/core';
import {
  AppWindow,
  ArrowUpToLine,
  Check,
  ChevronRight,
  Ellipsis,
  Folder,
  FolderInput,
  FolderOpen,
  FolderPlus,
  GripVertical,
  LayoutList,
  Lock,
  Pencil,
  Puzzle,
  RotateCcw,
  Trash2,
  Undo2,
  type LucideIcon,
} from 'lucide-react';
import { useI18n } from '../i18n';
import {
  MAX_PLUGIN_NAV_NAME,
  createPluginNavFolder,
  deletePluginNavFolder,
  emptyPluginNavArrangement,
  movePluginPageToFolder,
  movePluginPageToTop,
  placePluginNavItem,
  pluginNavDropPlan,
  renamePluginNavFolder,
  renamePluginPage,
  returnPluginPage,
  type PluginNavArrangement,
  type PluginNavDragItem,
  type PluginNavDropPlan,
  type PluginNavDropTarget,
  type ResolvedPluginNav,
  type ResolvedPluginPage,
} from '../services/pluginNavArrangement';
import type { PluginNavArrangementStatus } from '../services/pluginNavArrangementStore';
import type { PluginNavLayout } from '../services/pluginNavigation';

export const PLUGIN_CHILDREN_ID = 'nav-plugin-children';
const FOLDER_STATES_KEY = 'cpa-gui.plugin-nav-collapsed-folders';
const PLUGINS_KEY = 'plugins';

type MenuItem =
  | { separator: true; key: string }
  | {
    separator?: false;
    key: string;
    label: string;
    icon: LucideIcon;
    danger?: boolean;
    disabled?: boolean;
    hint?: string;
    movesFocus?: boolean;
    onSelect?: () => void;
    submenu?: MenuItem[];
  };
type MenuTarget = { kind: 'page'; page: ResolvedPluginPage } | { kind: 'folder'; id: string; name: string } | { kind: 'plugins' };
type OpenMenu = { target: MenuTarget; key: string; x: number; y: number };
type Renaming = { kind: 'page'; path: string } | { kind: 'folder'; id: string } | null;

type DragData = { item: PluginNavDragItem; label: string };
type DropData = { target: PluginNavDropTarget };
type RowHandle = { attributes: DraggableAttributes; listeners: ReturnType<typeof useDraggable>['listeners'] };

const pageKey = (path: string) => `page:${path}`;
const folderKey = (id: string) => `folder:${id}`;
const emptyKey = (id: string) => `empty:${id}`;
const targetKey = (target: PluginNavDropTarget) =>
  target.kind === 'plugins' ? PLUGINS_KEY : target.kind === 'folder' ? folderKey(target.id) : target.kind === 'empty' ? emptyKey(target.folder) : pageKey(target.path);
const isFolderState = (value: unknown): value is PluginNavLayout => value === 'collapsed' || value === 'expanded';

function readFolderStates(): Map<string, PluginNavLayout> {
  try {
    const parsed: unknown = JSON.parse(window.localStorage.getItem(FOLDER_STATES_KEY) ?? '{}');
    const entries = typeof parsed === 'object' && parsed !== null ? Object.entries(parsed) : [];
    return new Map(entries.filter((entry): entry is [string, PluginNavLayout] => isFolderState(entry[1])));
  } catch {
    return new Map();
  }
}

export function usePluginFolderStates() {
  const [states, setStates] = useState<ReadonlyMap<string, PluginNavLayout>>(readFolderStates);
  const setFolderState = useCallback((id: string, value: PluginNavLayout) => {
    setStates((current) => {
      if (current.get(id) === value) return current;
      const next = new Map(current).set(id, value);
      try { window.localStorage.setItem(FOLDER_STATES_KEY, JSON.stringify(Object.fromEntries(next))); } catch {}
      return next;
    });
  }, []);
  return [states, setFolderState] as const;
}

const rowKeyboardCoordinates: KeyboardCoordinateGetter = (event, { context: { active, collisionRect, droppableRects } }) => {
  if (event.code !== 'ArrowDown' && event.code !== 'ArrowUp') return undefined;
  event.preventDefault();
  if (!active || !collisionRect) return undefined;
  const down = event.code === 'ArrowDown';
  const center = collisionRect.top + collisionRect.height / 2;
  const middle = (rect: { top: number; height: number }) => rect.top + rect.height / 2;
  const next = [...droppableRects.entries()]
    .filter(([id, rect]) => id !== active.id && (down ? middle(rect) > center + 1 : middle(rect) < center - 1))
    .map(([, rect]) => rect)
    .sort((a, b) => (down ? a.top - b.top : b.top - a.top))[0];
  if (!next) return undefined;
  return { x: collisionRect.left, y: next.top + next.height * (down ? 0.75 : 0.2) - collisionRect.height / 2 };
};

function dropPlanFor(active: Active, over: Over | null): PluginNavDropPlan | null {
  const drag = active.data.current as DragData | undefined;
  const drop = over?.data.current as DropData | undefined;
  const rect = active.rect.current.translated;
  if (!drag || !drop || !over || !rect || over.rect.height <= 0) return null;
  const ratio = Math.min(Math.max((rect.top + rect.height / 2 - over.rect.top) / over.rect.height, 0), 1);
  return pluginNavDropPlan(drag.item, drop.target, ratio);
}

function NavRow({ id, drag, label, target, organizing, indicator, className, onContextMenu, children }: {
  id: string;
  drag: PluginNavDragItem | null;
  label: string;
  target: PluginNavDropTarget;
  organizing: boolean;
  indicator?: PluginNavDropPlan['position'];
  className: string;
  onContextMenu?: (event: ReactMouseEvent<HTMLElement>) => void;
  children: (handle: RowHandle | null) => ReactNode;
}) {
  const { t } = useI18n();
  const { setNodeRef: setDropRef } = useDroppable({ id, data: { target } satisfies DropData, disabled: !organizing });
  const { setNodeRef: setDragRef, attributes, listeners, isDragging } = useDraggable({
    id,
    data: drag ? { item: drag, label } satisfies DragData : undefined,
    disabled: !organizing || !drag,
    attributes: { roleDescription: t('app.nav.layout.dragRole') },
  });
  const ref = useCallback((node: HTMLDivElement | null) => {
    setDropRef(node);
    setDragRef(node);
  }, [setDropRef, setDragRef]);
  return (
    <div
      ref={ref}
      className={[className, isDragging ? 'dragging' : ''].filter(Boolean).join(' ')}
      data-plugin-nav-key={id}
      data-drop={indicator}
      onContextMenu={onContextMenu}
    >
      {children(organizing && drag ? { attributes, listeners } : null)}
    </div>
  );
}

function EmptyFolderRow({ id, organizing, indicator }: { id: string; organizing: boolean; indicator?: PluginNavDropPlan['position'] }) {
  const { t } = useI18n();
  const { setNodeRef } = useDroppable({ id: emptyKey(id), data: { target: { kind: 'empty', folder: id } } satisfies DropData, disabled: !organizing });
  return (
    <p ref={setNodeRef} className="nav-plugin-empty" data-plugin-nav-key={emptyKey(id)} data-drop={indicator}>
      {t(organizing ? 'app.nav.layout.dropHere' : 'app.nav.layout.emptyFolder')}
    </p>
  );
}

export function PluginNavToggle({ expanded, controls, label, onToggle }: {
  expanded: boolean;
  controls: string;
  label: string;
  onToggle: () => void;
}) {
  return (
    <button
      type="button"
      className="nav-plugin-toggle"
      aria-expanded={expanded}
      aria-controls={controls}
      aria-label={label}
      onClick={onToggle}
    >
      <ChevronRight size={14} strokeWidth={1.75} aria-hidden="true" />
    </button>
  );
}

function PluginNavMenu({ items, x, y, onClose }: { items: MenuItem[]; x: number; y: number; onClose: (restoreFocus: boolean) => void }) {
  const mainRef = useRef<HTMLDivElement>(null);
  const subRef = useRef<HTMLDivElement>(null);
  const [position, setPosition] = useState({ left: x, top: y });
  const [sub, setSub] = useState<{ index: number; focus: boolean } | null>(null);
  const [subPosition, setSubPosition] = useState({ left: 0, top: 0 });
  const subItems = sub ? (items[sub.index] as { submenu?: MenuItem[] }).submenu ?? [] : [];

  useLayoutEffect(() => {
    const menu = mainRef.current;
    if (!menu) return;
    const rect = menu.getBoundingClientRect();
    setPosition({
      left: Math.max(8, Math.min(x, window.innerWidth - rect.width - 8)),
      top: Math.max(8, Math.min(y, window.innerHeight - rect.height - 8)),
    });
    menu.querySelector<HTMLButtonElement>('button:not(:disabled)')?.focus();
  }, [x, y]);

  useLayoutEffect(() => {
    const menu = subRef.current;
    const anchor = sub ? mainRef.current?.querySelector<HTMLElement>(`[data-menu-index="${sub.index}"]`) : null;
    if (!sub || !menu || !anchor) return;
    const rect = menu.getBoundingClientRect();
    const trigger = anchor.getBoundingClientRect();
    const right = trigger.right + 4;
    setSubPosition({
      left: Math.max(8, right + rect.width > window.innerWidth - 8 ? trigger.left - rect.width - 4 : right),
      top: Math.max(8, Math.min(trigger.top - 4, window.innerHeight - rect.height - 8)),
    });
    if (sub.focus) menu.querySelector<HTMLButtonElement>('button:not(:disabled)')?.focus();
  }, [sub]);

  useEffect(() => {
    const closeFromOutside = (event: PointerEvent) => {
      const target = event.target as Node;
      if (!mainRef.current?.contains(target) && !subRef.current?.contains(target)) onClose(false);
    };
    const close = () => onClose(false);
    document.addEventListener('pointerdown', closeFromOutside, true);
    window.addEventListener('resize', close);
    window.addEventListener('blur', close);
    return () => {
      document.removeEventListener('pointerdown', closeFromOutside, true);
      window.removeEventListener('resize', close);
      window.removeEventListener('blur', close);
    };
  }, [onClose]);

  const choose = (item: MenuItem, index: number) => {
    if (item.separator || item.disabled) return;
    if (item.submenu) {
      setSub((current) => current?.index === index ? null : { index, focus: true });
      return;
    }
    onClose(!item.movesFocus);
    item.onSelect?.();
  };

  const handleKeyDown = (submenu: boolean) => (event: ReactKeyboardEvent<HTMLDivElement>) => {
    event.stopPropagation();
    const buttons = Array.from(event.currentTarget.querySelectorAll<HTMLButtonElement>('button:not(:disabled)'));
    const current = buttons.indexOf(document.activeElement as HTMLButtonElement);
    if (['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) {
      event.preventDefault();
      const next = event.key === 'Home' ? 0
        : event.key === 'End' ? buttons.length - 1
          : event.key === 'ArrowDown' ? (current + 1) % buttons.length
            : (current <= 0 ? buttons.length : current) - 1;
      buttons[next]?.focus();
    } else if (event.key === 'ArrowRight' && !submenu) {
      const index = Number((document.activeElement as HTMLElement | null)?.dataset.menuIndex);
      const item = items[index];
      if (item && !item.separator && item.submenu) {
        event.preventDefault();
        setSub({ index, focus: true });
      }
    } else if ((event.key === 'ArrowLeft' || event.key === 'Escape') && submenu) {
      event.preventDefault();
      const index = sub?.index;
      setSub(null);
      mainRef.current?.querySelector<HTMLButtonElement>(`[data-menu-index="${index}"]`)?.focus();
    } else if (event.key === 'Escape' || event.key === 'Tab') {
      event.preventDefault();
      onClose(true);
    }
  };

  const renderItems = (list: MenuItem[]) => list.map((item, index) => {
    if (item.separator) return <hr key={item.key} role="separator" />;
    const Icon = item.icon;
    return (
      <Fragment key={item.key}>
        <button
          type="button"
          role="menuitem"
          className={item.danger ? 'danger' : undefined}
          data-menu-index={index}
          disabled={item.disabled}
          aria-haspopup={item.submenu ? 'menu' : undefined}
          aria-expanded={item.submenu ? sub?.index === index : undefined}
          onClick={() => choose(item, index)}
        >
          <Icon size={14} strokeWidth={1.75} aria-hidden="true" />
          <span>{item.label}</span>
          {item.submenu ? <ChevronRight size={12} strokeWidth={1.75} aria-hidden="true" /> : null}
        </button>
        {item.hint ? <p className="plugin-nav-menu-hint">{item.hint}</p> : null}
      </Fragment>
    );
  });

  return createPortal(
    <>
      <div ref={mainRef} className="plugin-nav-menu" role="menu" style={position} onKeyDown={handleKeyDown(false)}>
        {renderItems(items)}
      </div>
      {sub ? (
        <div ref={subRef} className="plugin-nav-menu" role="menu" style={subPosition} onKeyDown={handleKeyDown(true)}>
          {renderItems(subItems)}
        </div>
      ) : null}
    </>,
    document.body,
  );
}

function RenameInput({ value, label, onDone }: { value: string; label: string; onDone: (value: string | null, refocus: boolean) => void }) {
  const done = useRef(false);
  const finish = (next: string | null, refocus: boolean) => {
    if (done.current) return;
    done.current = true;
    onDone(next, refocus);
  };
  return (
    <input
      className="nav-plugin-rename"
      defaultValue={value}
      aria-label={label}
      maxLength={MAX_PLUGIN_NAV_NAME}
      autoFocus
      onFocus={(event) => event.currentTarget.select()}
      onKeyDown={(event) => {
        if (event.nativeEvent.isComposing || event.keyCode === 229) return;
        if (event.key === 'Enter') {
          event.preventDefault();
          finish(event.currentTarget.value, true);
        } else if (event.key === 'Escape') {
          event.preventDefault();
          event.stopPropagation();
          finish(null, true);
        }
      }}
      onBlur={(event) => finish(event.currentTarget.value, false)}
    />
  );
}

export function PluginNavTree({
  nav,
  status,
  parentRow,
  pluginsExpanded,
  onTogglePlugins,
  active,
  locked,
  compact,
  collapsedFolders,
  onFolderCollapsed,
  organizing,
  onOrganize,
  onSelect,
  onChange,
}: {
  nav: ResolvedPluginNav;
  status: PluginNavArrangementStatus;
  parentRow: ReactNode;
  pluginsExpanded: boolean;
  onTogglePlugins: () => void;
  active: string;
  locked: boolean;
  compact: boolean;
  collapsedFolders: ReadonlySet<string>;
  onFolderCollapsed: (id: string, collapsed: boolean) => void;
  organizing: boolean;
  onOrganize: (organizing: boolean) => void;
  onSelect: (route: string) => void;
  onChange: (change: (arrangement: PluginNavArrangement) => PluginNavArrangement) => void;
}) {
  const { t } = useI18n();
  const root = useRef<HTMLDivElement>(null);
  const [menu, setMenu] = useState<OpenMenu | null>(null);
  const [renaming, setRenaming] = useState<Renaming>(null);
  const [dragging, setDragging] = useState<DragData | null>(null);
  const [plan, setPlan] = useState<PluginNavDropPlan | null>(null);
  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 4 } }),
    useSensor(KeyboardSensor, { coordinateGetter: rowKeyboardCoordinates }),
  );
  const editable = status === 'ready' && !compact;
  const hasPluginPages = nav.plugins.length > 0;
  const folderDrag = dragging?.item.kind === 'folder';
  const indicatorKey = plan ? targetKey(plan.target) : null;
  const indicator = (key: string) => (key === indicatorKey ? plan?.position : undefined);

  const focusRow = useCallback((key: string, control: 'more' | 'main') => {
    requestAnimationFrame(() => {
      const rows = root.current?.closest('.nav-section');
      const row = rows?.querySelector(`[data-plugin-nav-key="${CSS.escape(key)}"]`)
        ?? rows?.querySelector(`[data-plugin-nav-key="${PLUGINS_KEY}"]`);
      const target = control === 'more' ? row?.querySelector<HTMLElement>('.nav-plugin-more') : null;
      (target ?? row?.querySelector<HTMLElement>('button:not(.nav-plugin-toggle, .nav-plugin-more)'))?.focus();
    });
  }, []);

  const wasOrganizing = useRef(organizing);
  useEffect(() => {
    if (organizing === wasOrganizing.current) return;
    wasOrganizing.current = organizing;
    setMenu(null);
    setRenaming(null);
    requestAnimationFrame(() => {
      const section = root.current?.closest('.nav-section');
      const target = organizing
        ? section?.querySelector<HTMLElement>('[aria-roledescription]')
        : section?.querySelector<HTMLElement>(`[data-plugin-nav-key="${PLUGINS_KEY}"] > button:first-child`);
      target?.focus();
    });
  }, [organizing]);

  const menuRef = useRef(menu);
  menuRef.current = menu;
  const closeMenu = useCallback((restoreFocus: boolean) => {
    const current = menuRef.current;
    setMenu(null);
    if (current && restoreFocus) focusRow(current.key, 'more');
  }, [focusRow]);

  const openMenu = (target: MenuTarget, key: string, x: number, y: number) => {
    if (organizing || (!editable && !(target.kind === 'plugins' && status === 'unreadable' && !compact))) return;
    setRenaming(null);
    setMenu({ target, key, x, y });
  };

  const openFromButton = (target: MenuTarget, key: string) => (event: ReactMouseEvent<HTMLButtonElement>) => {
    const rect = event.currentTarget.getBoundingClientRect();
    if (menu?.key === key) closeMenu(false);
    else openMenu(target, key, rect.left, rect.bottom + 4);
  };

  const openFromContext = (target: MenuTarget, key: string) => (event: ReactMouseEvent<HTMLElement>) => {
    if (organizing) {
      event.preventDefault();
      return;
    }
    if (compact || status === 'loading' || (!editable && target.kind !== 'plugins')) return;
    event.preventDefault();
    openMenu(target, key, event.clientX, event.clientY);
  };

  const newFolder = (path?: string) => {
    const id = crypto.randomUUID();
    onChange((current) => createPluginNavFolder(current, id, t('app.nav.layout.newFolder'), path));
    onFolderCollapsed(id, false);
    setRenaming({ kind: 'folder', id });
  };

  const menuItems = (target: MenuTarget): MenuItem[] => {
    if (status === 'unreadable') {
      return [{ key: 'unreadable', label: t('app.nav.layout.loadFailed'), icon: RotateCcw, disabled: true }];
    }
    if (target.kind === 'page') {
      const { path } = target.page.entry;
      const inTop = nav.custom.some((node) => node.kind === 'page' && node.page.entry.path === path);
      const inCustom = !nav.plugins.some((page) => page.entry.path === path);
      const targets = nav.custom.flatMap((node) => node.kind === 'folder' && !node.pages.some((page) => page.entry.path === path) ? [node] : []);
      const items: MenuItem[] = [{ key: 'rename', label: t('app.nav.layout.rename'), icon: Pencil, movesFocus: true, onSelect: () => setRenaming({ kind: 'page', path }) }];
      if (!inTop) items.push({ key: 'top', label: t('app.nav.layout.moveToTop'), icon: ArrowUpToLine, onSelect: () => onChange((current) => movePluginPageToTop(current, path)) });
      items.push({
        key: 'folder',
        label: t('app.nav.layout.moveToFolder'),
        icon: FolderInput,
        submenu: [
          ...targets.map((folder): MenuItem => ({
            key: folderKey(folder.id),
            label: folder.name,
            icon: Folder,
            onSelect: () => {
              onChange((current) => movePluginPageToFolder(current, path, folder.id));
              onFolderCollapsed(folder.id, false);
            },
          })),
          ...(targets.length ? [{ separator: true as const, key: 'separator' }] : []),
          { key: 'new', label: t('app.nav.layout.newFolderWith'), icon: FolderPlus, movesFocus: true, onSelect: () => newFolder(path) },
        ],
      });
      if (inCustom) items.push({ key: 'plugins', label: t('app.nav.layout.moveToPlugins'), icon: Puzzle, onSelect: () => onChange((current) => returnPluginPage(current, path)) });
      if (target.page.renamed) {
        items.push(
          { separator: true, key: 'separator' },
          {
            key: 'restore',
            label: t('app.nav.layout.restoreName', { name: target.page.entry.label }),
            icon: Undo2,
            onSelect: () => onChange((current) => renamePluginPage(current, path, '', target.page.entry.label)),
          },
        );
      }
      return items;
    }
    if (target.kind === 'folder') {
      return [
        { key: 'rename', label: t('app.nav.layout.rename'), icon: Pencil, movesFocus: true, onSelect: () => setRenaming({ kind: 'folder', id: target.id }) },
        { separator: true, key: 'separator' },
        {
          key: 'delete',
          label: t('app.nav.layout.deleteFolder'),
          icon: Trash2,
          danger: true,
          hint: t('app.nav.layout.deleteFolderHint'),
          onSelect: () => onChange((current) => deletePluginNavFolder(current, target.id)),
        },
      ];
    }
    return [
      { key: 'new', label: t('app.nav.layout.newFolder'), icon: FolderPlus, movesFocus: true, onSelect: () => newFolder() },
      { key: 'organize', label: t('app.nav.layout.organize'), icon: LayoutList, movesFocus: true, onSelect: () => onOrganize(true) },
      { separator: true, key: 'separator' },
      { key: 'reset', label: t('app.nav.layout.resetLayout'), icon: RotateCcw, onSelect: () => onChange(() => emptyPluginNavArrangement()) },
    ];
  };

  const moreButton = (target: MenuTarget, key: string, name: string) => (
    <button
      type="button"
      className="nav-plugin-more"
      aria-label={t('app.nav.layout.actions', { name })}
      aria-haspopup="menu"
      aria-expanded={menu?.key === key}
      onClick={openFromButton(target, key)}
    >
      <Ellipsis size={16} strokeWidth={1.75} aria-hidden="true" />
    </button>
  );

  const grip = organizing ? <GripVertical size={14} strokeWidth={1.75} className="nav-plugin-grip" aria-hidden="true" /> : null;

  const finishRename = (key: string, apply: (value: string) => void) => (value: string | null, refocus: boolean) => {
    if (value !== null) apply(value);
    setRenaming(null);
    if (refocus) focusRow(key, 'main');
  };

  const pageRow = (page: ResolvedPluginPage, folder: string | null, inPlugins: boolean) => {
    const { entry } = page;
    const key = pageKey(entry.path);
    const isActive = entry.route === active;
    const isRenaming = renaming?.kind === 'page' && renaming.path === entry.path;
    const rowLocked = locked && !organizing;
    const title = rowLocked ? t('app.nav.lockedHint') : page.renamed ? `${entry.label} · ${entry.pluginTitle}` : entry.description;
    return (
      <NavRow
        key={key}
        id={key}
        drag={isRenaming ? null : { kind: 'page', path: entry.path, inPlugins }}
        label={page.label}
        target={{ kind: 'page', path: entry.path, folder, inPlugins }}
        organizing={organizing}
        indicator={indicator(key)}
        className="nav-plugin-row"
        onContextMenu={openFromContext({ kind: 'page', page }, key)}
      >
        {(handle) => (
          <>
            {isRenaming ? (
              <div className="nav-plugin-rename-row">
                <AppWindow size={16} strokeWidth={1.75} aria-hidden="true" />
                <RenameInput
                  value={page.label}
                  label={t('app.nav.layout.nameInput')}
                  onDone={finishRename(key, (value) => onChange((current) => renamePluginPage(current, entry.path, value, entry.label)))}
                />
              </div>
            ) : (
              <button
                type="button"
                className={['nav-plugin-entry', isActive ? 'active' : '', rowLocked ? 'locked' : ''].filter(Boolean).join(' ')}
                aria-current={isActive ? 'page' : undefined}
                data-plugin-route={entry.route}
                disabled={rowLocked}
                title={title}
                onClick={organizing ? undefined : () => onSelect(entry.route)}
                {...handle?.attributes}
                {...handle?.listeners}
              >
                {grip}
                <AppWindow size={16} strokeWidth={1.75} aria-hidden="true" />
                <span>{page.label}</span>
                {rowLocked ? <Lock size={12} strokeWidth={1.75} className="nav-lock-icon" aria-hidden="true" /> : null}
              </button>
            )}
            {editable && !organizing && !isRenaming ? moreButton({ kind: 'page', page }, key, page.label) : null}
          </>
        )}
      </NavRow>
    );
  };

  const finishDrag = () => {
    setDragging(null);
    setPlan(null);
  };

  const handleDragStart = ({ active: dragged }: DragStartEvent) => {
    setMenu(null);
    setRenaming(null);
    setDragging((dragged.data.current as DragData | undefined) ?? null);
  };

  const handleDragMove = ({ active: dragged, over }: DragMoveEvent) => {
    const next = dropPlanFor(dragged, over);
    setPlan((current) => (JSON.stringify(current) === JSON.stringify(next) ? current : next));
  };

  const handleDragEnd = ({ active: dragged, over, activatorEvent }: DragEndEvent) => {
    const next = dropPlanFor(dragged, over);
    const drag = dragged.data.current as DragData | undefined;
    finishDrag();
    if (!next || !drag) return;
    onChange((current) => placePluginNavItem(current, drag.item, next.placement));
    if (activatorEvent instanceof KeyboardEvent) focusRow(String(dragged.id), 'main');
  };

  const dragName = (dragged: Active) => (dragged.data.current as DragData | undefined)?.label ?? '';
  const announcements: Announcements = {
    onDragStart: ({ active: dragged }) => t('app.nav.layout.dragStart', { name: dragName(dragged) }),
    onDragOver: () => undefined,
    onDragEnd: ({ active: dragged }) => t('app.nav.layout.dragEnd', { name: dragName(dragged) }),
    onDragCancel: ({ active: dragged }) => t('app.nav.layout.dragCancel', { name: dragName(dragged) }),
  };

  return (
    <div ref={root} className="nav-plugin-tree">
      <DndContext
        sensors={sensors}
        collisionDetection={closestCenter}
        measuring={{ droppable: { strategy: MeasuringStrategy.Always } }}
        accessibility={{ announcements, screenReaderInstructions: { draggable: t('app.nav.layout.dragInstructions') } }}
        onDragStart={handleDragStart}
        onDragMove={handleDragMove}
        onDragOver={handleDragMove}
        onDragEnd={handleDragEnd}
        onDragCancel={finishDrag}
      >
        {nav.custom.map((node, index) => {
          if (node.kind === 'page') return pageRow(node.page, null, false);
          const key = folderKey(node.id);
          const groupId = `nav-plugin-folder-${index}`;
          const expanded = !folderDrag && (organizing || compact || !collapsedFolders.has(node.id));
          const containsActive = node.pages.some((page) => page.entry.route === active);
          const isRenaming = renaming?.kind === 'folder' && renaming.id === node.id;
          const FolderIcon = expanded ? FolderOpen : Folder;
          const toggle = () => onFolderCollapsed(node.id, expanded);
          return (
            <Fragment key={key}>
              <NavRow
                id={key}
                drag={isRenaming ? null : { kind: 'folder', id: node.id }}
                label={node.name}
                target={{ kind: 'folder', id: node.id }}
                organizing={organizing}
                indicator={indicator(key)}
                className={['nav-plugin-row', 'nav-plugin-folder', organizing ? '' : 'has-toggle'].filter(Boolean).join(' ')}
                onContextMenu={openFromContext({ kind: 'folder', id: node.id, name: node.name }, key)}
              >
                {(handle) => (
                  <>
                    {isRenaming ? (
                      <div className="nav-plugin-rename-row">
                        <FolderIcon size={16} strokeWidth={1.75} aria-hidden="true" />
                        <RenameInput
                          value={node.name}
                          label={t('app.nav.layout.nameInput')}
                          onDone={finishRename(key, (value) => onChange((current) => renamePluginNavFolder(current, node.id, value)))}
                        />
                      </div>
                    ) : (
                      <button
                        type="button"
                        className={['nav-plugin-folder-head', !expanded && containsActive ? 'active' : ''].filter(Boolean).join(' ')}
                        aria-expanded={expanded}
                        aria-controls={groupId}
                        onClick={organizing ? undefined : toggle}
                        {...handle?.attributes}
                        {...handle?.listeners}
                      >
                        {grip}
                        <FolderIcon size={16} strokeWidth={1.75} aria-hidden="true" />
                        <span>{node.name}</span>
                      </button>
                    )}
                    {editable && !organizing && !isRenaming ? moreButton({ kind: 'folder', id: node.id, name: node.name }, key, node.name) : null}
                    {organizing ? null : (
                      <PluginNavToggle expanded={expanded} controls={groupId} label={t('app.nav.layout.toggleFolder', { name: node.name })} onToggle={toggle} />
                    )}
                  </>
                )}
              </NavRow>
              {expanded ? (
                <div id={groupId} className="nav-plugin-children" role="group" aria-label={node.name}>
                  {node.pages.length
                    ? node.pages.map((page) => pageRow(page, node.id, false))
                    : <EmptyFolderRow id={node.id} organizing={organizing} indicator={indicator(emptyKey(node.id))} />}
                </div>
              ) : null}
            </Fragment>
          );
        })}
        <NavRow
          id={PLUGINS_KEY}
          drag={null}
          label={t('app.nav.plugins')}
          target={{ kind: 'plugins' }}
          organizing={organizing}
          indicator={indicator(PLUGINS_KEY)}
          className={['nav-plugin-parent', 'nav-plugin-row', hasPluginPages && !organizing ? 'has-toggle' : ''].filter(Boolean).join(' ')}
          onContextMenu={openFromContext({ kind: 'plugins' }, PLUGINS_KEY)}
        >
          {() => (
            <>
              {parentRow}
              {status === 'loading' || compact || organizing ? null : moreButton({ kind: 'plugins' }, PLUGINS_KEY, t('app.nav.plugins'))}
              {hasPluginPages && !organizing ? (
                <PluginNavToggle expanded={pluginsExpanded} controls={PLUGIN_CHILDREN_ID} label={t('app.nav.pluginPages')} onToggle={onTogglePlugins} />
              ) : null}
            </>
          )}
        </NavRow>
        {pluginsExpanded && hasPluginPages ? (
          <div id={PLUGIN_CHILDREN_ID} className="nav-plugin-children" role="group" aria-label={t('app.nav.pluginPages')}>
            {nav.plugins.map((page) => pageRow(page, null, true))}
          </div>
        ) : null}
        <DragOverlay dropAnimation={null}>
          {dragging ? (
            <div className="nav-plugin-drag-overlay">
              <GripVertical size={14} strokeWidth={1.75} className="nav-plugin-grip" aria-hidden="true" />
              {dragging.item.kind === 'folder'
                ? <Folder size={16} strokeWidth={1.75} aria-hidden="true" />
                : <AppWindow size={16} strokeWidth={1.75} aria-hidden="true" />}
              <span>{dragging.label}</span>
            </div>
          ) : null}
        </DragOverlay>
      </DndContext>
      {organizing ? (
        <div className="nav-plugin-organize-bar">
          <button type="button" className="nav-plugin-organize-new" onClick={() => newFolder()}>
            <FolderPlus size={14} strokeWidth={1.75} aria-hidden="true" />
            <span>{t('app.nav.layout.newFolder')}</span>
          </button>
          <button type="button" className="nav-plugin-organize-done" onClick={() => onOrganize(false)}>
            <Check size={14} strokeWidth={1.75} aria-hidden="true" />
            <span>{t('app.nav.layout.done')}</span>
          </button>
        </div>
      ) : null}
      {menu ? <PluginNavMenu items={menuItems(menu.target)} x={menu.x} y={menu.y} onClose={closeMenu} /> : null}
    </div>
  );
}
