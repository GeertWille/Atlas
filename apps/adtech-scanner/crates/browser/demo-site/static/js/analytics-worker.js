/* Meridian audience analytics: fetches the sampling rules and hands them to the page. */
fetch('/collect/sampling.json')
  .then(function (answer) {
    return answer.json();
  })
  .then(function (rules) {
    self.postMessage(rules);
  })
  .catch(function () {});
