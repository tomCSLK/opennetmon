// "Sensor hinzufügen": shows only the fields of the chosen sensor type (fieldsets with data-kinds).
// Without the script all fields stay visible; the server ignores the ones of other types.
(function () {
  'use strict';
  var kind = document.getElementById('kind');
  if (!kind) { return; }
  var sets = document.querySelectorAll('fieldset[data-kinds]');
  function show() {
    for (var i = 0; i < sets.length; i++) {
      sets[i].hidden = sets[i].getAttribute('data-kinds').split(' ').indexOf(kind.value) < 0;
    }
  }
  kind.addEventListener('change', show);
  show();
})();
