module Main
  ( main
  ) where

import Awesome.Prelude
import Funky (funkify)

main :: IO ()
main = funkify >> pure ()
