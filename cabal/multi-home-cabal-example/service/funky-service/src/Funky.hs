module Funky
  ( funkify
  ) where

import Awesome.Prelude
import Gnarly (gnarlify)

funkify :: IO Integer
funkify = pure number
  where
    number = either (const 0) id $ gnarlify "foobar"
