(define-constant err-already-registered u1)
(define-constant err-not-registered u2)
(define-map stackers { user: principal } { added-on-block-height: uint })

(define-read-only (is-stacker (user principal))
  (ok (is-some (map-get? stackers { user: user })))
)

(define-read-only (is-not-stacker (user principal))
  (ok (is-none (map-get? stackers { user: user })))
)

(define-public (register)
  (if (unwrap-panic (is-not-stacker tx-sender))
    (ok (map-set stackers { user: tx-sender } { added-on-block-height: block-height }))
    (err err-already-registered)
  )
)

(define-public (unregister)
  (if (unwrap-panic (is-stacker tx-sender))
    (ok (map-delete stackers { user: tx-sender }))
    (err err-not-registered)
  )
)
