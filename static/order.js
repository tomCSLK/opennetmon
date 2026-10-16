// Order pages: drag rows by their handle, move them with the ↑/↓ buttons or type a position.
// The position fields are what the form submits; this script only keeps them in sync.
(() => {
  'use strict';

  function renumber(list) {
    [...list.children].forEach((li, i) => {
      const input = li.querySelector('input.pos');
      if (input) input.value = String(i + 1);
    });
  }

  function setup(list) {
    list.classList.add('js');
    let dragged = null;

    list.addEventListener('click', (event) => {
      const button = event.target.closest('button[data-move]');
      if (!button) return;
      const li = button.closest('li');
      if (button.dataset.move === 'up' && li.previousElementSibling) {
        list.insertBefore(li, li.previousElementSibling);
      } else if (button.dataset.move === 'down' && li.nextElementSibling) {
        list.insertBefore(li.nextElementSibling, li);
      }
      renumber(list);
      button.focus();
    });

    // A typed position moves the row there at once.
    list.addEventListener('change', (event) => {
      const input = event.target.closest('input.pos');
      if (!input) return;
      const li = input.closest('li');
      const others = [...list.children].filter((x) => x !== li);
      let n = Number.parseInt(input.value, 10);
      if (!Number.isFinite(n)) n = others.length + 1;
      n = Math.min(Math.max(n, 1), others.length + 1);
      list.insertBefore(li, others[n - 1] || null);
      renumber(list);
      input.focus();
    });

    // Rows are only draggable while the handle is held, so text in the fields stays selectable.
    list.addEventListener('pointerdown', (event) => {
      const handle = event.target.closest('.handle');
      if (handle) handle.closest('li').draggable = true;
    });
    list.addEventListener('pointerup', () => {
      if (!dragged) [...list.children].forEach((li) => { li.draggable = false; });
    });
    list.addEventListener('dragstart', (event) => {
      dragged = event.target.closest('li');
      if (!dragged) return;
      dragged.classList.add('dragging');
      event.dataTransfer.effectAllowed = 'move';
      event.dataTransfer.setData('text/plain', '');
    });
    list.addEventListener('dragover', (event) => {
      if (!dragged) return;
      event.preventDefault();
      const over = event.target.closest('li');
      if (!over || over === dragged || over.parentNode !== list) return;
      const box = over.getBoundingClientRect();
      const after = event.clientY > box.top + box.height / 2;
      list.insertBefore(dragged, after ? over.nextElementSibling : over);
    });
    list.addEventListener('drop', (event) => event.preventDefault());
    list.addEventListener('dragend', () => {
      if (!dragged) return;
      dragged.classList.remove('dragging');
      dragged.draggable = false;
      dragged = null;
      renumber(list);
    });
  }

  document.querySelectorAll('ol.order-list').forEach(setup);
})();
