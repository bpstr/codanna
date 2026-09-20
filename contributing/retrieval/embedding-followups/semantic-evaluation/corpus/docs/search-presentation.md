# Search presentation

Waystation formats each retrieved passage as a source link, section title, and short excerpt. The excerpt emphasizes exact words shared by the question and passage when such words exist. A semantic match can have no emphasized words at all, especially across languages.

The presentation layer may shorten a long excerpt for the available panel width. Expanding that excerpt reveals more text from the same retrieved passage; it does not issue a larger nearest-neighbor request. The panel's row height and the maximum highlighted-word count are independent display settings.

The formatter does not decide similarity eligibility, change stored passages, or remove obsolete source revisions. A broken excerpt or highlight can be investigated independently from the collection's result count and from the writer's update transaction.
