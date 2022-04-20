module Awesome.Prelude
  ( UUID
  , module X
  ) where

--------------------------------------------------------------------------------
import Prelude as X hiding (appendFile, error, foldl, head, print, putChar, putStr, putStrLn, readFile, show, tail, take, unlines, unwords, words, writeFile, (!!))

--------------------------------------------------------------------------------
import Control.Lens as X (_Left, _Right, over, preview, set, to, view, (%~), (&), (.~), (?~), (^.))
import Control.Lens as X ((^?))
import Control.Lens.Extras as X (is)

--------------------------------------------------------------------------------
import Control.Applicative as X ((<|>))
import Control.Monad as X (guard, join, unless, when, (<=<), (>=>))
import Control.Monad.Except as X (ExceptT (..), MonadError (..), liftEither, runExceptT)
import Control.Monad.Except as X (withExceptT)
import Control.Monad.IO.Unlift as X
import Control.Monad.Reader as X (MonadReader (..), ReaderT (..), asks)
import Control.Monad.Trans.Class as X (MonadTrans (lift))
import Control.Monad.Trans.Maybe as X (MaybeT (..), runMaybeT)

--------------------------------------------------------------------------------
import Data.ByteString as X (ByteString)

--------------------------------------------------------------------------------
import Data.Foldable as X (for_, toList, traverse_)
import Data.Functor as X (void, ($>), (<&>))
import Data.Generics.Labels as X ()
import Data.Int as X (Int64)
import Data.List as X (find, sort)
import Data.List.NonEmpty as X (NonEmpty)
import Data.Maybe as X (catMaybes, fromMaybe, isJust, isNothing)
import Data.Proxy as X (Proxy (..))
import Data.String as X (IsString (..))
import Data.Time as X (Day, UTCTime)
import Data.Traversable as X (for)
import Data.Typeable as X (Typeable)
import Data.UUID as UUID (UUID)
import GHC.Generics as X (Generic)

--------------------------------------------------------------------------------
import Data.Text as X (Text, pack, take, toUpper, unlines, unpack, unwords, words)
import Data.Text.Encoding as X (decodeUtf8', encodeUtf8)
import Data.Text.IO as X (appendFile, readFile, writeFile)
import Data.Text.Lens as X (packed, unpacked)
