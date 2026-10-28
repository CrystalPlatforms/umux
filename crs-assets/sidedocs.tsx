import React, { isValidElement, useLayoutEffect, useRef, useState, type CSSProperties, type ReactNode } from 'react';
import { HugeiconsIcon, type IconSvgElement } from '@hugeicons/react';
import {
  CursorPointer01Icon,
  Download04Icon,
  Layers01Icon,
  Notification03Icon,
  PaintBoardIcon,
  Rocket01Icon,
  Settings02Icon,
  TextFontIcon
} from '@hugeicons/core-free-icons';
import './BranchedMenu.css';

export interface BranchedMenuChild {
  value: string;
  label: string;
  icon?: ReactNode | IconSvgElement;
}

export interface BranchedMenuItem {
  label: string;
  value?: string;
  children?: BranchedMenuChild[];
}

export interface BranchedMenuProps {
  items?: BranchedMenuItem[];
  defaultOpen?: number | number[];
  defaultActive?: string;
  onSelect?: (value: string, item: BranchedMenuChild | BranchedMenuItem) => void;
  onToggle?: (index: number, open: boolean) => void;
  color?: string;
  accentColor?: string;
  lineColor?: string;
  width?: number;
  rowHeight?: number;
  indent?: number;
  trunk?: number;
  radius?: number;
  lineWidth?: number;
  fontSize?: number;
  drawDuration?: number;
  foldDuration?: number;
  className?: string;
}

const DEFAULT_ITEMS: BranchedMenuItem[] = [
  {
    label: 'Getting started',
    children: [
      { value: 'install', label: 'Installation', icon: Download04Icon },
      { value: 'quick', label: 'Quick start', icon: Rocket01Icon },
      { value: 'config', label: 'Configuration', icon: Settings02Icon },
      { value: 'theming', label: 'Theming', icon: PaintBoardIcon }
    ]
  },
  {
    label: 'Components',
    children: [
      { value: 'buttons', label: 'Buttons', icon: CursorPointer01Icon },
      { value: 'typography', label: 'Typography', icon: TextFontIcon },
      { value: 'overlays', label: 'Overlays', icon: Layers01Icon },
      { value: 'toasts', label: 'Toasts', icon: Notification03Icon }
    ]
  }
];
const PAD = 6;
const MARK = 16;

const renderIcon = (icon: ReactNode | IconSvgElement) =>
  isValidElement(icon) ? icon : <HugeiconsIcon icon={icon as IconSvgElement} size={16} strokeWidth={1.8} />;
const toSet = (open: number | number[]) => new Set(Array.isArray(open) ? open : open >= 0 ? [open] : []);

const BranchedMenu: React.FC<BranchedMenuProps> = ({
  items = DEFAULT_ITEMS,
  defaultOpen = 0,
  defaultActive = '',
  onSelect,
  onToggle,
  color = '#f5f5f5',
  accentColor = '#f5f5f5',
  lineColor = '#3f3f46',
  width = 240,
  rowHeight = 36,
  indent = 40,
  trunk = 14,
  radius = 10,
  lineWidth = 1.5,
  fontSize = 14,
  drawDuration = 400,
  foldDuration = 300,
  className = ''
}) => {
  const [open, setOpen] = useState<Set<number>>(() => toSet(defaultOpen));
  const [active, setActive] = useState(() => {
    if (defaultActive) return defaultActive;
    const first = items.find((it, i) => it.children && toSet(defaultOpen).has(i));
    return first?.children?.[0]?.value ?? '';
  });
  const navRef = useRef<HTMLElement>(null);
  const heads = useRef<(HTMLButtonElement | null)[]>([]);
  const markerRef = useRef<HTMLSpanElement>(null);
  const latest = useRef<{ onSelect?: BranchedMenuProps['onSelect']; onToggle?: BranchedMenuProps['onToggle'] }>({});
  latest.current = { onSelect, onToggle };

  const activeSection = items.findIndex(it => it.children?.some(kid => kid.value === active));
  const markerShown = activeSection >= 0 && open.has(activeSection);
  useLayoutEffect(() => {
    const place = (glide: boolean) => {
      const m = markerRef.current;
      const el = heads.current[activeSection];
      if (!m) return;
      const on = markerShown && el;
      if (!glide) m.style.transition = 'none';
      if (on) m.style.top = `${el.offsetTop + (el.offsetHeight - MARK) / 2}px`;
      m.toggleAttribute('data-on', Boolean(on));
      if (!glide) {
        void m.offsetHeight;
        m.style.transition = '';
      }
    };
    place(true);
    let first = true;
    const ro = new ResizeObserver(() => {
      if (first) {
        first = false;
        return;
      }
      place(false);
    });
    if (navRef.current) ro.observe(navRef.current);
    return () => ro.disconnect();
  }, [activeSection, markerShown, items, fontSize, rowHeight]);

  const select = (value: string, item: BranchedMenuChild | BranchedMenuItem) => {
    setActive(value);
    latest.current.onSelect?.(value, item);
  };
  const toggle = (i: number) => {
    setOpen(prev => {
      const next = new Set(prev);
      const isOpen = !next.has(i);
      if (isOpen) next.add(i);
      else next.delete(i);
      latest.current.onToggle?.(i, isOpen);
      return next;
    });
  };

  const r = Math.min(radius, rowHeight / 2 - 2);
  const endX = indent - 8;
  const rowY = (k: number) => PAD + k * rowHeight + rowHeight / 2;
  const branch = (k: number) => `M ${trunk} ${rowY(k) - r} A ${r} ${r} 0 0 0 ${trunk + r} ${rowY(k)} H ${endX}`;
  const reach = (k: number) => `M ${trunk} 0 V ${rowY(k) - r} A ${r} ${r} 0 0 0 ${trunk + r} ${rowY(k)} H ${endX}`;
  const length = (k: number) => rowY(k) - r + (Math.PI * r) / 2 + (endX - trunk - r);

  return (
    <nav
      ref={navRef}
      className={`branched-menu${className ? ` ${className}` : ''}`}
      style={
        {
          '--bm-w': `${width}px`,
          '--bm-ink': color,
          '--bm-accent': accentColor,
          '--bm-line': lineColor,
          '--bm-font': `${fontSize}px`,
          '--bm-row': `${rowHeight}px`,
          '--bm-indent': `${indent}px`,
          '--bm-line-w': lineWidth,
          '--bm-draw': `${drawDuration}ms`,
          '--bm-fold': `${foldDuration}ms`
        } as CSSProperties
      }
    >
      <span ref={markerRef} className="branched-menu__marker" aria-hidden="true" />
      {items.map((item, i) => {
        const kids = item.children;
        const isOpen = kids ? open.has(i) : false;
        const leafValue = item.value ?? item.label;
        const leafActive = !kids && leafValue === active;
        const bodyH = kids ? PAD * 2 + kids.length * rowHeight : 0;
        return (
          <div key={item.value ?? item.label} className="branched-menu__section" data-open={isOpen ? '' : undefined}>
            <button
              ref={el => {
                heads.current[i] = el;
              }}
              type="button"
              className="branched-menu__head"
              aria-expanded={kids ? isOpen : undefined}
              aria-current={leafActive ? 'true' : undefined}
              data-active={leafActive ? '' : undefined}
              onClick={() => (kids ? toggle(i) : select(leafValue, item))}
            >
              {item.label}
            </button>
            {kids ? (
              <div className="branched-menu__body">
                <div className="branched-menu__fold">
                  <div className="branched-menu__tree" style={{ height: bodyH }}>
                    <svg className="branched-menu__lines" width={indent} height={bodyH} aria-hidden="true">
                      <path className="branched-menu__base" d={`M ${trunk} 0 V ${rowY(kids.length - 1) - r}`} />
                      {kids.map((kid, k) => (
                        <path key={kid.value} className="branched-menu__base" d={branch(k)} />
                      ))}
                      {kids.map((kid, k) => (
                        <path
                          key={kid.value}
                          className="branched-menu__reach"
                          d={reach(k)}
                          style={{
                            strokeDasharray: length(k),
                            strokeDashoffset: kid.value === active ? 0 : length(k)
                          }}
                        />
                      ))}
                    </svg>
                    {kids.map(kid => (
                      <button
                        key={kid.value}
                        type="button"
                        className="branched-menu__item"
                        aria-current={kid.value === active ? 'true' : undefined}
                        data-active={kid.value === active ? '' : undefined}
                        tabIndex={isOpen ? 0 : -1}
                        onClick={() => select(kid.value, kid)}
                      >
                        {kid.icon ? (
                          <span className="branched-menu__icon" aria-hidden="true">
                            {renderIcon(kid.icon)}
                          </span>
                        ) : null}
                        <span className="branched-menu__label">{kid.label}</span>
                      </button>
                    ))}
                  </div>
                </div>
              </div>
            ) : null}
          </div>
        );
      })}
    </nav>
  );
};

export default BranchedMenu;


/* ---- undefined.css ---- */
.branched-menu {
  --bm-w: 240px;
  --bm-ink: #f5f5f5;
  --bm-accent: #f5f5f5;
  --bm-line: #3f3f46;
  --bm-font: 14px;
  --bm-row: 36px;
  --bm-indent: 40px;
  --bm-line-w: 1.5;
  --bm-draw: 400ms;
  --bm-fold: 300ms;
  --bm-ease-out: cubic-bezier(0.23, 1, 0.32, 1);
  --bm-muted: color-mix(in srgb, var(--bm-ink) 55%, transparent);

  position: relative;
  display: flex;
  flex-direction: column;
  width: fit-content;
  max-width: min(var(--bm-w), 100%);
  padding-left: 14px;
  color: var(--bm-ink);
  font-family: inherit;
  font-size: var(--bm-font);
  line-height: 1.2;
}

.branched-menu::before {
  content: '';
  position: absolute;
  top: 8px;
  bottom: 0;
  left: 0;
  width: 2px;
  border-radius: 1px;
  background: linear-gradient(to bottom, var(--bm-line) 0%, var(--bm-line) 55%, transparent 100%);
}

.branched-menu__marker {
  position: absolute;
  top: -1px;
  left: 0;
  z-index: 1;
  width: 2px;
  height: 16px;
  border-radius: 1px;
  background: var(--bm-accent);
  opacity: 0;
  transition:
    top 220ms var(--bm-ease-out),
    opacity 150ms ease;
}

.branched-menu__marker[data-on] {
  opacity: 1;
}

.branched-menu__section {
  display: flex;
  flex-direction: column;
}

.branched-menu__head {
  display: block;
  margin: 0;
  padding: 9px 0;
  border: 0;
  background: none;
  color: var(--bm-muted);
  font: inherit;
  font-size: calc(var(--bm-font) + 1px);
  font-weight: 500;
  text-align: left;
  cursor: pointer;
  outline: none;
  -webkit-tap-highlight-color: transparent;
  transition: color 200ms ease;
}

.branched-menu__section[data-open] .branched-menu__head,
.branched-menu__head[data-active] {
  color: var(--bm-ink);
}

@media (hover: hover) and (pointer: fine) {
  .branched-menu__head:hover {
    color: var(--bm-ink);
  }
}

.branched-menu__body {
  display: grid;
  grid-template-rows: 0fr;
  transition: grid-template-rows var(--bm-fold) var(--bm-ease-out);
}

.branched-menu__section[data-open] .branched-menu__body {
  grid-template-rows: 1fr;
}

.branched-menu__fold {
  min-height: 0;
  overflow: hidden;
}

.branched-menu__tree {
  position: relative;
  padding: 6px 0;
  box-sizing: border-box;
}

.branched-menu__lines {
  position: absolute;
  top: 0;
  left: 0;
  overflow: visible;
  opacity: 0;
  pointer-events: none;
  transition: opacity 200ms ease;
}

.branched-menu__section[data-open] .branched-menu__lines {
  opacity: 1;
  transition: opacity 250ms ease 100ms;
}

.branched-menu__base,
.branched-menu__reach {
  fill: none;
  stroke-width: var(--bm-line-w);
  stroke-linecap: round;
  stroke-linejoin: round;
}

.branched-menu__base {
  stroke: var(--bm-line);
}

.branched-menu__reach {
  stroke: var(--bm-accent);
  transition: stroke-dashoffset var(--bm-draw) var(--bm-ease-out);
}

.branched-menu__item {
  display: flex;
  align-items: center;
  gap: 8px;
  width: 100%;
  height: var(--bm-row);
  margin: 0;
  padding: 0 0 0 var(--bm-indent);
  border: 0;
  background: none;
  color: var(--bm-muted);
  font: inherit;
  text-align: left;
  cursor: pointer;
  outline: none;
  box-sizing: border-box;
  -webkit-tap-highlight-color: transparent;
  transition: color 200ms ease;
}

@media (hover: hover) and (pointer: fine) {
  .branched-menu__item:hover {
    color: var(--bm-ink);
  }
}

.branched-menu__item[data-active] {
  color: var(--bm-accent);
  font-weight: 500;
}

.branched-menu__icon {
  display: inline-flex;
  flex: none;
}

.branched-menu__label {
  white-space: nowrap;
}

@media (prefers-reduced-motion: reduce) {
  .branched-menu__marker {
    transition: opacity 150ms ease;
  }

  .branched-menu__body {
    transition: none;
  }

  .branched-menu__reach {
    transition: none;
  }
}
