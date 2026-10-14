import {
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent,
} from 'react';
import { Check, ChevronDown, LoaderCircle, RefreshCw, Search, X } from 'lucide-react';
import {
  agentModelAlias,
  filterAgentModels,
  findAgentModel,
} from '../services/agentModelPicker';
import type { ModelOption } from '../services/modelService';
import { useI18n } from '../i18n';

export type AgentModelPickerProps = {
  models: ModelOption[];
  value: string;
  loading?: boolean;
  error?: string;
  disabled?: boolean;
  onChange: (value: string) => void;
  onRefresh?: () => void;
  allowCustomValue?: boolean;
  editable?: { label: string; placeholder: string; maxLength: number };
  menuClassName?: string;
  emptyOption?: string;
  preserveValue?: boolean;
};

type AgentModelDropdownLayout = {
  top: number;
  left: number;
  width: number;
  height: number;
};

export function AgentModelPicker({
  models,
  value,
  loading = false,
  error = '',
  disabled = false,
  onChange,
  onRefresh,
  allowCustomValue = false,
  editable,
  menuClassName = '',
  emptyOption,
  preserveValue = false,
}: AgentModelPickerProps) {
  const { t } = useI18n();
  const [open, setOpen] = useState(false);
  const [search, setSearch] = useState('');
  const [activeIndex, setActiveIndex] = useState(0);
  const [dropdownLayout, setDropdownLayout] = useState<AgentModelDropdownLayout | null>(null);
  const [dialogOverlay, setDialogOverlay] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const searchRef = useRef<HTMLInputElement>(null);
  const valueRef = useRef<HTMLInputElement>(null);
  const listboxId = useId();
  const visibleModels = useMemo(
    () => editable && !search.trim() ? models : filterAgentModels(models, search),
    [models, search, editable],
  );
  const choices = useMemo(() => {
    const options = visibleModels.map((model) => ({ name: model.name, alias: model.alias ?? '' }));
    if (emptyOption && !search.trim()) options.unshift({ name: '', alias: emptyOption });
    if (allowCustomValue && search.trim() && !findAgentModel(models, search)) {
      options.push({ name: search.trim(), alias: t('agents.model.useCustom') });
    }
    return options;
  }, [visibleModels, allowCustomValue, search, models, t, emptyOption]);
  const selectedModel = findAgentModel(models, value);
  const selectedName = (selectedModel?.name
    ?? (allowCustomValue || editable || preserveValue ? value.trim() : ''))
    || (emptyOption && !value.trim() ? emptyOption : '');
  const selectedAlias = selectedName ? agentModelAlias(models, selectedName) : '';

  const updateDropdownLayout = useCallback(() => {
    const root = rootRef.current;
    if (!root) return;

    const rect = root.getBoundingClientRect();
    const dialog = root.closest('dialog');
    const footer = dialog?.querySelector('.credential-settings-footer');
    const heading = dialog?.querySelector('.credential-settings-heading');
    const frame = dialog?.getBoundingClientRect();
    const footerTop = footer?.getBoundingClientRect().top;
    const headingBottom = heading?.getBoundingClientRect().bottom;
    const edgeGap = 12;
    const triggerGap = 6;
    const preferredHeight = 282;
    const minimumHeight = 150;
    const bounds = {
      top: (headingBottom ?? frame?.top ?? 0) + edgeGap,
      bottom: (footerTop ?? frame?.bottom ?? window.innerHeight) - edgeGap,
      left: (frame?.left ?? 0) + edgeGap,
      right: (frame?.right ?? window.innerWidth) - edgeGap,
    };
    const scroller = dialog?.querySelector('.credential-settings-body');
    if (scroller) {
      const view = scroller.getBoundingClientRect();
      if (rect.bottom < view.top + 4 || rect.top > view.bottom - 4) {
        setOpen(false);
        return;
      }
    }
    const spaceBelow = Math.max(0, bounds.bottom - rect.bottom - triggerGap);
    const spaceAbove = Math.max(0, rect.top - triggerGap - bounds.top);
    const placeAbove = spaceBelow < minimumHeight && spaceAbove > spaceBelow;
    const availableHeight = placeAbove ? spaceAbove : spaceBelow;
    const limit = Math.max(0, bounds.bottom - bounds.top);
    const height = Math.min(preferredHeight, limit, Math.max(Math.min(minimumHeight, limit), availableHeight));
    const width = Math.min(rect.width, Math.max(0, bounds.right - bounds.left));
    const left = Math.min(Math.max(bounds.left, rect.left), Math.max(bounds.left, bounds.right - width));
    const desiredTop = placeAbove ? rect.top - triggerGap - height : rect.bottom + triggerGap;
    const top = Math.min(Math.max(bounds.top, desiredTop), Math.max(bounds.top, bounds.bottom - height));

    setDropdownLayout({ top, left, width, height });
  }, []);

  useLayoutEffect(() => {
    if (!open) {
      setDropdownLayout(null);
      setDialogOverlay(false);
      return undefined;
    }
    const inDialog = Boolean(rootRef.current?.closest('dialog'));
    if (inDialog !== dialogOverlay) {
      setDialogOverlay(inDialog);
      return undefined;
    }

    updateDropdownLayout();
    const menu = menuRef.current;
    if (dialogOverlay && menu && !menu.matches(':popover-open')) menu.showPopover();
    const onScroll = (event: Event) => {
      if (menuRef.current?.contains(event.target as Node)) return;
      updateDropdownLayout();
    };
    window.addEventListener('resize', updateDropdownLayout);
    window.addEventListener('scroll', onScroll, true);
    return () => {
      window.removeEventListener('resize', updateDropdownLayout);
      window.removeEventListener('scroll', onScroll, true);
    };
  }, [open, dialogOverlay, updateDropdownLayout]);

  useEffect(() => {
    if (!open) return undefined;
    const close = (event: MouseEvent) => {
      const target = event.target as Node;
      if (!rootRef.current?.contains(target) && !menuRef.current?.contains(target)) setOpen(false);
    };
    document.addEventListener('mousedown', close);
    return () => document.removeEventListener('mousedown', close);
  }, [open]);

  useEffect(() => {
    if (!open) return;
    if (!editable) setSearch('');
    const selectedIndex = choices.findIndex(
      (model) => model.name.toLocaleLowerCase() === value.trim().toLocaleLowerCase(),
    );
    setActiveIndex(selectedIndex >= 0 ? selectedIndex : 0);
    if (!editable) requestAnimationFrame(() => searchRef.current?.focus());
  }, [open]);

  useEffect(() => {
    setActiveIndex((current) => Math.min(current, Math.max(choices.length - 1, 0)));
  }, [choices.length]);

  const choose = (name: string) => {
    onChange(name);
    setOpen(false);
    if (editable) valueRef.current?.focus();
  };

  const moveActive = (offset: number) => {
    if (choices.length === 0) return;
    setActiveIndex((current) => (current + offset + choices.length) % choices.length);
  };

  const handleSearchKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === 'ArrowDown') {
      event.preventDefault();
      if (!open) setOpen(true);
      else moveActive(1);
    } else if (event.key === 'ArrowUp') {
      event.preventDefault();
      if (!open) setOpen(true);
      else moveActive(-1);
    } else if (event.key === 'Enter' && open && choices[activeIndex]) {
      event.preventDefault();
      choose(choices[activeIndex].name);
    } else if (event.key === 'Escape') {
      event.preventDefault();
      setOpen(false);
    }
  };

  return (
    <div className={`agent-model-picker ${open ? 'open' : ''}`} ref={rootRef}
      onMouseDown={(event) => {
        if (event.button === 0 && event.currentTarget.contains(document.activeElement)
          && event.target instanceof Element && event.target.closest('button')) {
          event.preventDefault();
        }
      }}
      onBlur={(event) => {
        const next = event.relatedTarget as Node | null;
        if (!event.currentTarget.contains(next) && !menuRef.current?.contains(next)) setOpen(false);
      }}>
      {editable ? (
        <div className="agent-model-trigger agent-model-editable-trigger">
          <input ref={valueRef} type="text" value={value} aria-label={editable.label}
            placeholder={editable.placeholder} maxLength={editable.maxLength} disabled={disabled}
            spellCheck={false} autoComplete="off" role="combobox" aria-autocomplete="list"
            aria-controls={listboxId} aria-expanded={open}
            onClick={() => { setSearch(''); setOpen(true); }} onKeyDown={handleSearchKeyDown}
            onChange={(event) => {
              onChange(event.currentTarget.value);
              setSearch(event.currentTarget.value);
              setActiveIndex(0);
              setOpen(true);
            }} />
          <button type="button" className="icon-button quiet" aria-label={editable.label}
            aria-haspopup="listbox" aria-expanded={open} aria-controls={listboxId}
            disabled={disabled} onClick={() => { setSearch(''); setOpen((current) => !current); }}>
            <ChevronDown size={17} aria-hidden />
          </button>
        </div>
      ) : <button
        type="button"
        className="agent-model-trigger"
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-controls={listboxId}
        disabled={disabled}
        onClick={() => setOpen((current) => !current)}
        onKeyDown={(event) => {
          if (!open && ['ArrowDown', 'ArrowUp', 'Enter', ' '].includes(event.key)) {
            event.preventDefault();
            setOpen(true);
          }
        }}
      >
        <span>
          <strong title={selectedName || undefined}>
            {selectedName || (loading ? t('agents.model.loading') : error ? t('agents.model.loadFailed') : models.length ? t('agents.model.select') : t('agents.model.none'))}
          </strong>
          {selectedAlias ? <small title={selectedAlias}>{selectedAlias}</small> : null}
        </span>
        <ChevronDown size={17} aria-hidden />
      </button>}

      {open ? (
        <div
          ref={menuRef}
          popover={dialogOverlay ? 'manual' : undefined}
          className={`agent-model-dropdown${editable ? ' agent-model-dropdown-editable' : ''}${menuClassName ? ` ${menuClassName}` : ''}`}
          data-theme-surface="overlay"
          style={dropdownLayout
            ? { top: dropdownLayout.top, left: dropdownLayout.left, width: dropdownLayout.width, height: dropdownLayout.height }
            : { top: 0, left: 0, width: 0, height: 0, visibility: 'hidden' }}
        >
          {!editable ? <div className="agent-model-search">
            <Search size={15} aria-hidden />
            <input
              ref={searchRef}
              value={search}
              onChange={(event) => {
                setSearch(event.currentTarget.value);
                setActiveIndex(0);
              }}
              onKeyDown={handleSearchKeyDown}
              placeholder={t('agents.model.search')}
              role="combobox"
              aria-controls={listboxId}
              aria-expanded="true"
            />
            {search ? (
              <button
                type="button"
                className="icon-button quiet"
                onClick={() => {
                  setSearch('');
                  setActiveIndex(0);
                  searchRef.current?.focus();
                }}
                title={t('agents.model.clearSearch')}
                aria-label={t('agents.model.clearSearch')}
              >
                <X size={14} aria-hidden="true" />
              </button>
            ) : null}
            {onRefresh ? <button type="button" className="icon-button quiet" onClick={onRefresh} disabled={loading} title={t('agents.model.refresh')} aria-label={t('agents.model.refresh')}>
              <RefreshCw size={14} className={loading ? 'spin' : ''} aria-hidden="true" />
            </button> : null}
          </div> : null}

          <div className="agent-model-list" id={listboxId} role="listbox">
            {loading && models.length === 0 && choices.length === 0 ? (
              <div className="agent-model-empty"><LoaderCircle size={18} className="spin" />{t('agents.model.fetching')}</div>
            ) : error && models.length === 0 && choices.length === 0 ? (
              <div className="agent-model-empty error"><strong>{t('agents.model.loadFailed')}</strong><span>{error}</span></div>
            ) : choices.length === 0 ? (
              <div className="agent-model-empty">
                {editable ? <span>{t('agents.claudeDesktopMapping.customAliasHint')}</span> : <>
                  <strong>{search.trim() ? t('agents.model.noMatch') : t('agents.model.unavailable')}</strong>
                  <span>{search.trim() ? t('agents.model.tryKeywords') : t('agents.model.connectFirst')}</span>
                </>}
              </div>
            ) : choices.map((choice, index) => {
              const selected = choice.name.toLocaleLowerCase() === value.trim().toLocaleLowerCase();
              return (
                <button
                  type="button"
                  role="option"
                  aria-selected={selected}
                  className={`agent-model-option ${selected ? 'selected' : ''} ${index === activeIndex ? 'active' : ''}`}
                  key={choice.name}
                  onMouseEnter={() => setActiveIndex(index)}
                  onClick={() => choose(choice.name)}
                >
                  <span>
                    <strong title={choice.name}>{choice.name || emptyOption}</strong>
                    {choice.name ? <small>{choice.alias || t('agents.model.available')}</small> : null}
                  </span>
                  {selected ? <Check size={16} aria-hidden /> : null}
                </button>
              );
            })}
          </div>
          <div className="agent-model-dropdown-footer">
            <span>{t(editable ? 'agents.claudeDesktopMapping.suggestionCount' : 'agents.model.count', { count: models.length })}</span>
            {error && models.length > 0 ? <span className="error">{t('agents.model.stale')}</span> : null}
          </div>
        </div>
      ) : null}
    </div>
  );
}
