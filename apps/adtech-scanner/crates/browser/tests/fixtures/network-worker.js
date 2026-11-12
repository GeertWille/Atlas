fetch('/status/200?worker-' + self.location.search.slice(1)).then(function (answer) { return answer.text(); });
